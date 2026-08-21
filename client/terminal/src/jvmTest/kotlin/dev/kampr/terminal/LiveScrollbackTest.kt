package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.SplitDirection
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.render.LogicalText
import dev.kampr.terminal.render.SurfaceRows
import dev.kampr.terminal.review.ReviewSurface
import dev.kampr.terminal.review.historyWarning
import dev.kampr.terminal.view.TerminalView
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue
import kotlin.test.fail

private val URL: String? = System.getenv("KAMPR_URL")
private val TOKEN: String? = System.getenv("KAMPR_TOKEN")
private val REQUIRED: Boolean = System.getenv("KAMPR_LIVE") != null

private fun live(): Endpoint? {
    val url = URL
    val token = TOKEN
    if (!url.isNullOrBlank() && !token.isNullOrBlank()) return Endpoint(url, token)
    val why = "LiveScrollbackTest SKIPPED — KAMPR_URL/KAMPR_TOKEN unset, no real scrollback was seen"
    if (REQUIRED) fail("$why, and KAMPR_LIVE demanded one")
    System.err.println("\n${"!".repeat(78)}\n  $why\n${"!".repeat(78)}\n")
    return null
}

private class LiveIo(private val store: KamprStore) : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override fun info(paneId: String): PaneInfo? = store.paneInfo(paneId)
}

private fun warningFor(pane: PaneState): String? {
    val rows = SurfaceRows(pane)
    return historyWarning(ReviewSurface(rows, LogicalText(rows), 0))
}

private suspend fun <T : Any> await(what: String, poll: () -> T?): T {
    repeat(900) {
        poll()?.let { return it }
        delay(50)
    }
    error("never saw $what")
}

// The node has always reported `capped` and `complete` honestly and the surface threw both away,
// so a hole in the record looked exactly like a pane that had said less. Hand-built Scrollback
// objects would have agreed with whatever the client believed; these frames come off the wire.
@OptIn(ExperimentalTestApi::class)
class LiveScrollbackTest {
    @Test
    fun aRealRingsGapsAndCapsReachTheSurface() {
        val target = live() ?: return
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        lateinit var clipped: PaneState
        lateinit var broken: PaneState
        var workspace: String? = null
        try {
            runBlocking {
                connection.connect(target)
                await("hello") { store.hello.value }
                val node = await("a herd") {
                    store.herd.value.takeIf { it.known }?.nodes?.firstOrNull { it.kind == "local" }
                }.id

                // Its own workspace, so both rings start empty. Reusing whatever panes happen to be
                // open makes the second run of this test read a ring the first run already broke.
                connection.manage(ManageOp.WorkspaceCreate(node, "kampr-scrollback", "/tmp"))
                val created = await("the workspace") {
                    store.managed.value?.takeIf { it.op == "workspace.create" && it.ok }
                }
                workspace = assertNotNull(created.id)
                val first = await("its pane") {
                    store.herd.value.panes.firstOrNull { it.workspaceId == workspace }
                }
                connection.manage(ManageOp.PaneSplit(first.id, SplitDirection.Right, 0.5))
                await("the split") { store.managed.value?.takeIf { it.op == "pane.split" && it.ok } }
                val second = await("the second pane") {
                    store.herd.value.panes.firstOrNull { it.workspaceId == workspace && it.id != first.id }
                }

                // Fill one past herdr's 1000-line read before anything watches it. The node's ring
                // then starts at index 0 and still cannot reach what came before: `capped` with
                // `complete`, which are not the same fact and do not mean the same loss.
                connection.send(ClientMsg.InputText(second.id, "seq 1 4000\r"))
                delay(4_000)
                connection.watch(second.id)
                clipped = store.pane(second.id)
                await("a ring that was already against the cap when it was first read") {
                    Unit.takeIf { clipped.scrollback.capped && clipped.scrollback.complete }
                }
                val note = assertNotNull(warningFor(clipped), "a clipped ring said nothing")
                assertTrue(note.contains("1000"), note)

                // The other pane is watched from empty, and stays honest while its history is small.
                connection.watch(first.id)
                broken = store.pane(first.id)
                await("the grid") { Unit.takeIf { broken.painted } }
                connection.send(ClientMsg.InputText(first.id, "seq 1 120\r"))
                await("a short, whole history") { Unit.takeIf { broken.scrollback.historyRows > 60 } }
                assertNull(warningFor(broken), "an intact ring must say nothing at all")

                // Now outrun the poll. The node discards rather than splicing across the gap, so
                // from_top advances and `complete` goes false with a real count behind it.
                connection.send(ClientMsg.InputText(first.id, "seq 1 9000\r"))
                await("the node discarding across a gap") {
                    Unit.takeIf { !broken.scrollback.complete && broken.scrollback.fromTop > 0 }
                }
                val lost = assertNotNull(warningFor(broken))
                assertTrue(lost.contains("${broken.scrollback.fromTop} rows"), lost)
            }
        } finally {
            runBlocking {
                workspace?.let {
                    connection.manage(ManageOp.Close(it))
                    delay(1_000)
                }
            }
            connection.disconnect()
            scope.cancel()
        }

        val io = LiveIo(store)
        runComposeUiTest {
            setContent {
                CompositionLocalProvider(LocalTokens provides liveTokens(), LocalPaneIo provides io) {
                    Box(Modifier.fillMaxSize()) { TerminalView(broken, PaneSession(broken.id), io) }
                }
            }
            onNodeWithContentDescription(
                "${broken.scrollback.fromTop} rows were discarded",
                substring = true,
            ).assertExists()
        }
        runComposeUiTest {
            setContent {
                CompositionLocalProvider(LocalTokens provides liveTokens(), LocalPaneIo provides io) {
                    Box(Modifier.fillMaxSize()) { TerminalView(clipped, PaneSession(clipped.id), io) }
                }
            }
            onNodeWithContentDescription("history is clipped", substring = true).assertExists()
            onAllNodesWithContentDescription("were discarded", substring = true).assertCountEquals(0)
        }
    }
}

private fun liveTokens(): KamprTokens {
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    return KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Phone))
}
