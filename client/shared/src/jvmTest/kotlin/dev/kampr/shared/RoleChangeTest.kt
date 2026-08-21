package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.LocalManage
import dev.kampr.shared.ui.ManageLayer
import dev.kampr.shared.ui.ManageIo
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Sheet
import dev.kampr.shared.ui.RoleNotice
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

// The frames a node actually puts on the wire for a mid-session demotion and the promotion back.
private const val DEMOTED = """{"t":"role","role":"readonly"}"""
private const val PROMOTED = """{"t":"role","role":"full"}"""

// `hello` from OutboxTest's stub says `manage: false`; the affordances this test is about need a
// node that offers them, so this one differs in that one field.
private const val HELLO_MANAGE =
    """{"build":"0.1.0","caps":{"conversation":true,"manage":true,"mesh":false,"push":true,""" +
        """"scrollback":true},"node_id":"01JNODE","node_name":"stub","protocol":1,"role":"full",""" +
        """"security":{"encrypted":true,"installable":true,"origin":"http://127.0.0.1",""" +
        """"passkeys":false,"push":true,"tier":0,"unencrypted_banner":false,"unlocks":[]},""" +
        """"t":"hello"}"""

private val INFO = PaneInfo(
    id = PANE,
    nodeId = "01JNODE",
    workspace = "kampr",
    agent = "claude",
    agentStatus = "idle",
    cols = 80,
    rows = 24,
    hasConversation = true,
)

private class Surfaces : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier)

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(1.dp))
}

// Exactly what `AppManage` does, so the test gates on the same value the app gates on.
private class StoreManage(private val store: KamprStore) : ManageIo {
    override val enabled: Boolean get() = store.canManage
    override fun openNew(paneId: String?) = Unit
    override fun openActions(paneId: String) = Unit
}

private fun tokens() = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

@OptIn(ExperimentalTestApi::class)
class RoleChangeTest {
    // The node enforces a mid-session demotion within two seconds and used to tell nobody, so the
    // key row, the New sheet and the manage actions stayed drawn on a device that could no longer
    // use any of them. A real socket, because a frame that only exists in a test fixture is a
    // frame both sides of the seam can agree about while neither agrees with the wire.
    @Test
    fun aRoleChangeOnALiveSocketReachesTheStore() {
        val port = freePort()
        val node = StubNode(port, HELLO_MANAGE)
        node.start()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        try {
            runBlocking {
                connection.connect(Endpoint("http://127.0.0.1:$port"))
                until("hello") { store.hello.value != null }
                assertFalse(store.readOnly, "the stub greeted this device as a writer")
                assertNull(store.roleNote, "nothing has changed yet")

                node.push(DEMOTED)
                until("the demotion") { store.readOnly }
                assertEquals("readonly", store.role, "the store is the one place the role lives")
                assertFalse(store.canManage, "a demoted device kept its manage affordances")
                assertEquals(
                    "full",
                    store.hello.value?.role,
                    "`hello` is the greeting this connection was given and must not be rewritten",
                )
                val note = assertNotNullNote(store.roleNote)
                assertTrue("read-only" in note, "a device that loses its buttons must be told why: $note")

                node.push(PROMOTED)
                until("the promotion") { !store.readOnly }
                assertEquals("full", store.role)
                assertTrue(store.canManage, "a promoted device had to reconnect to get its buttons back")
                assertTrue("write" in assertNotNullNote(store.roleNote), store.roleNote.orEmpty())
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }

    // The affordances themselves. `readOnly` was read once out of `hello` and off a StateFlow that
    // composition never subscribed to, so even a store that knew would have drawn the old screen.
    @Test
    fun theWriteAffordancesFollowTheRoleWithoutAReconnect() = runComposeUiTest {
        val store = KamprStore()
        store.take(HELLO_MANAGE)
        setContent {
            CompositionLocalProvider(
                LocalTokens provides tokens(),
                LocalManage provides StoreManage(store),
            ) {
                Box(Modifier.size(411.dp, 914.dp)) {
                    PaneScreenMobile(
                        pane = store.pane(PANE),
                        info = INFO,
                        view = PaneView.Terminal,
                        surfaces = Surfaces(),
                        landscape = false,
                        readOnly = store.readOnly,
                        onBack = {},
                        onView = {},
                        onAnswer = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                    store.roleNote?.let { RoleNotice(it, store::dismissRoleNote) }
                }
            }
        }
        waitForIdle()
        assertEquals(1, onAllNodesWithContentDescription(NEW).fetchSemanticsNodes().size)
        assertEquals(0, onAllNodesWithContentDescription(READ_ONLY).fetchSemanticsNodes().size)

        store.take(DEMOTED)
        waitForIdle()
        assertEquals(
            0,
            onAllNodesWithContentDescription(NEW).fetchSemanticsNodes().size,
            "the New action survived a demotion, so it is present-and-failing",
        )
        onNodeWithContentDescription(READ_ONLY).assertExists()
        // It appears without the operator having done anything, which is what the live region is
        // for — the same convention the watch notice and the error strip already use.
        onNodeWithContentDescription(store.roleNote.orEmpty(), substring = true).assertExists()

        store.take(PROMOTED)
        waitForIdle()
        assertEquals(1, onAllNodesWithContentDescription(NEW).fetchSemanticsNodes().size)
        assertEquals(0, onAllNodesWithContentDescription(READ_ONLY).fetchSemanticsNodes().size)
    }

    // The one write affordance that is already on screen when the role moves. `openSheet` refuses
    // a read-only device, and the sheet it opened a moment earlier had nothing telling it to go.
    @Test
    fun aSheetOpenedBeforeTheDemotionDoesNotOutliveIt() = runComposeUiTest {
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
        val store = KamprStore()
        store.take(HELLO_MANAGE)
        val app = AppState(scope, store, MemoryPrefs(), null)
        app.openSheet(Sheet.New("01JNODE", PANE))
        val herd = Herd(
            nodes = listOf(NodeInfo("01JNODE", "stub", "local")),
            panes = listOf(INFO),
            known = true,
        )
        try {
            setContent {
                CompositionLocalProvider(LocalTokens provides tokens()) {
                    Box(Modifier.size(411.dp, 914.dp)) {
                        ManageLayer(app, herd, Breakpoint.Portrait)
                    }
                }
            }
            waitForIdle()
            assertNotNull(app.sheet, "a writer's sheet must stay up")

            store.take(DEMOTED)
            waitForIdle()
            assertNull(app.sheet, "a New sheet outlived the role that allowed it")
        } finally {
            scope.cancel()
        }
    }
}

private const val NEW = "New, from this pane"
private const val READ_ONLY = "This device is read-only — it cannot type into the pane"

private fun assertNotNullNote(note: String?): String =
    note ?: error("the role change was never surfaced to the operator")
