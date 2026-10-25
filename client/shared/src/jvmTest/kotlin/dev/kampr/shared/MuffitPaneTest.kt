package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.saidOutLoud
import dev.kampr.shared.model.serves
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.Breakpoint
import dev.kampr.shared.ui.MenuAnchor
import dev.kampr.shared.ui.PaneActionsSheet
import dev.kampr.shared.ui.PaneCard
import dev.kampr.shared.ui.PaneMenu
import dev.kampr.shared.ui.PaneRow
import dev.kampr.shared.ui.PaneScreenDesktop
import dev.kampr.shared.ui.PaneScreenMobile
import dev.kampr.shared.ui.PaneSurfaces
import dev.kampr.shared.ui.PaneView
import dev.kampr.shared.ui.Screen
import dev.kampr.shared.ui.named
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val MUFFIT_ID = "01JNODE/m:s1"
private const val TERMINAL_SURFACE = "Terminal surface probe"
private const val CONVERSATION_SURFACE = "Conversation surface probe"
private const val ZOOM = "Zoom probe"
private const val TOOLS = "Tools probe"
private const val NO_PICTURE = "This pane has no picture"
private const val REFUSED = "stream_unavailable: kampr cannot run herdr"

private const val HERD = """
    {"t":"herd",
     "nodes":[{"id":"01JNODE","name":"comingclean","kind":"local","online":true,"native":true}],
     "panes":[
       {"id":"01JNODE/m:s1","node_id":"01JNODE","label":"tidy the parser","cwd":"/home/dbrain/dev/kampr",
        "agent":"muffit","provider":"muffit","agent_status":"working","rows":0,
        "has_conversation":true,"converses":true,"updated_at":"2026-10-25T09:00:00Z"}]}
"""

private fun muffit(): PaneInfo = (Wire.decode(HERD) as ServerMsg.Herd).panes.single()

private class Probe : PaneSurfaces {
    @Composable
    override fun Terminal(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier.named(TERMINAL_SURFACE))

    @Composable
    override fun Conversation(pane: PaneState, info: PaneInfo?, modifier: Modifier) = Box(modifier.named(CONVERSATION_SURFACE))

    @Composable
    override fun KeyRow(pane: PaneState, compact: Boolean, modifier: Modifier) = Box(modifier)

    @Composable
    override fun Zoom(pane: PaneState, modifier: Modifier) = Box(modifier.size(40.dp).named(ZOOM))
}

private fun neverPainted(): PaneState = PaneState(MUFFIT_ID, StyleTable()).apply { refusal = REFUSED }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.count(label: String): Int = onAllNodesWithContentDescription(label, substring = true, useUnmergedTree = true).fetchSemanticsNodes().size

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.texts(): List<String> = onAllNodesWithText("", substring = true)
    .fetchSemanticsNodes()
    .mapNotNull { node -> node.config.getOrElseNullable(SemanticsProperties.Text) { null }?.joinToString("") { it.text } }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.assertOnlyAConversation(where: String) {
    onNodeWithContentDescription(CONVERSATION_SURFACE).assertIsDisplayed()
    assertEquals(0, count(TERMINAL_SURFACE), "$where: a muffit session was given a terminal")
    for (tab in listOf("Terminal view", "Conversation view", "Split view")) {
        assertEquals(0, count(tab), "$where: a muffit session was offered \"$tab\"")
    }
    assertEquals(0, count(ZOOM), "$where: a muffit session was offered the zoom")
    assertEquals(0, count(NO_PICTURE), "$where: a muffit session was said to have failed to paint")
    assertEquals(0, count(REFUSED), "$where: a muffit session was said to have failed to paint")
    assertEquals(0, onAllNodesWithText(NO_PICTURE).fetchSemanticsNodes().size, "$where: stream notice")
}

private fun state(): Pair<AppState, CoroutineScope> {
    val scope = CoroutineScope(SupervisorJob() + Dispatchers.Unconfined)
    val store = KamprStore()
    store.accept(Wire.decode(HERD)!!)
    return AppState(scope, store, MemoryPrefs(), null).apply { go(Screen.Herd) } to scope
}

// docs/18: a muffit session has no terminal at all, so it is a conversation on every breakpoint,
// offers nothing that only a grid could use, and is never a terminal that failed to paint.
@OptIn(ExperimentalTestApi::class)
class MuffitPaneTest {
    @Test
    fun aDesktopOpensAMuffitSessionOnItsConversation() {
        val (app, scope) = state()
        try {
            app.compact = false
            app.openPane(MUFFIT_ID)
            assertEquals(PaneView.Conversation, (app.screen as Screen.Pane).view)
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun aViewRememberedForTheTerminalStillOpensTheConversation() {
        val (app, scope) = state()
        try {
            app.store.accept(Wire.decode("""{"t":"prefs","panes":{"$MUFFIT_ID":{"view":"terminal"}}}""")!!)
            app.openPane(MUFFIT_ID)
            assertEquals(PaneView.Conversation, (app.screen as Screen.Pane).view)
        } finally {
            scope.cancel()
        }
    }

    @Test
    fun theDesktopPaneIsTheConversationAndNothingElse() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(1280.dp, 860.dp)) {
                    PaneScreenDesktop(neverPainted(), muffit().copy(detail = REFUSED), PaneView.Terminal, Probe(), readOnly = false, onView = {}, modifier = Modifier.fillMaxSize())
                }
            }
        }
        waitForIdle()
        assertOnlyAConversation("desktop")
    }

    @Test
    fun aPhoneEitherWayRoundIsTheConversationAndNothingElse() {
        for ((size, landscape) in listOf((411.dp to 914.dp) to false, (914.dp to 411.dp) to true)) {
            runComposeUiTest { mobile(size.first, size.second, landscape) }
        }
    }

    @OptIn(ExperimentalTestApi::class)
    private fun ComposeUiTest.mobile(width: Dp, height: Dp, landscape: Boolean) {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(width, height)) {
                    PaneScreenMobile(
                        neverPainted(),
                        muffit().copy(detail = REFUSED),
                        PaneView.Split,
                        Probe(),
                        landscape = landscape,
                        readOnly = false,
                        onBack = {},
                        onView = {},
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        waitForIdle()
        assertOnlyAConversation(if (landscape) "landscape" else "portrait")
    }

    @Test
    fun theHerdMarksAMuffitSessionAndSaysSo() = runComposeUiTest {
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(420.dp, 200.dp)) {
                    Column {
                        PaneRow(muffit(), 0.0, false) {}
                        PaneCard(muffit(), 0.0, {})
                    }
                }
            }
        }
        waitForIdle()
        assertEquals(2, onAllNodesWithText("muffit", useUnmergedTree = true).fetchSemanticsNodes().size, "the row and the card are not marked: ${texts()}")
        assertEquals(2, count("Muffit session"), "the mark is not spoken")
    }

    @Test
    fun theListMenuClosesTheSessionAndOffersNothingHerdrsOrATerminals() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.size(900.dp, 600.dp)) {
                    PaneMenu(Breakpoint.Desktop, muffit(), MenuAnchor(120.dp, 120.dp), null, { sent += it }, {}, {}, desk = true)
                }
            }
        }
        waitForIdle()
        assertEquals(listOf("Open", "Close"), texts())
        onNodeWithText("Close").performClick()
        waitForIdle()
        assertTrue(sent.isEmpty(), "closed without asking: $sent")
        onNodeWithText("close").performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeClose(MUFFIT_ID)), sent)
    }

    @Test
    fun theInSessionSheetClosesTheSessionAndOffersNothingHerdrsOrATerminals() = runComposeUiTest {
        val sent = mutableListOf<ManageOp>()
        setContent {
            CompositionLocalProvider(LocalTokens provides phoneTokens()) {
                Box(Modifier.fillMaxSize()) {
                    PaneActionsSheet(
                        Breakpoint.Portrait,
                        muffit(),
                        null,
                        { sent += it },
                        {},
                        onFind = {},
                        tools = { Box(Modifier.size(40.dp).named(TOOLS)) },
                        desk = true,
                    )
                }
            }
        }
        waitForIdle()
        val shown = texts()
        for (absent in listOf("focus at the desk", "fill the tab", "remember the splits", "find", "rename", "tab", "workspace")) {
            assertFalse(absent in shown, "a muffit session's sheet offers \"$absent\": $shown")
        }
        assertEquals(0, count(TOOLS), "a muffit session's sheet offers the terminal's tools")
        onNodeWithText("close").performClick()
        waitForIdle()
        onAllNodesWithText("close")[1].performClick()
        waitForIdle()
        assertEquals(listOf<ManageOp>(ManageOp.NativeClose(MUFFIT_ID)), sent)
    }

    @Test
    fun aMuffitSessionNeedsOnlyItsNodeAndNotItsHerdr() {
        val cold = NodeInfo("01JNODE", "comingclean", "peer", online = false, reachable = true, native = true)
        assertTrue(cold.serves(muffit()))
        val muffitOnly = Wire.decode(
            """{"t":"herd","nodes":[{"id":"01JNODE","name":"comingclean","online":false,"reachable":true,"muffit":true}],"panes":[]}""",
        ) as ServerMsg.Herd
        assertTrue(muffitOnly.nodes.single().isServing, "a machine serving muffit sessions with no herdr read as gone")
        assertTrue(muffitOnly.nodes.single().serves(muffit()))
        assertFalse(cold.copy(native = null).isServing, "a machine with neither herdr nor sessions is still not serving")
        val herdr = Wire.decode("""{"t":"error","code":"herdr_unavailable","message":"herdr went","node":"01JNODE"}""") as ServerMsg.Failure
        assertFalse(saidOutLoud(herdr, MUFFIT_ID), "a herdr outage interrupted a muffit session")
    }
}
