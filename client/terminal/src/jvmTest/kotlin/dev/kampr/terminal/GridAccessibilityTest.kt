package dev.kampr.terminal

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.SemanticsMatcher
import androidx.compose.ui.test.assert
import androidx.compose.ui.test.assertHeightIsAtLeast
import androidx.compose.ui.test.isFocused
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.SoftTheme
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.terminal.guard.SubmitGuard
import dev.kampr.terminal.input.Esc
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.PaneKeyRow
import dev.kampr.terminal.view.ConfirmSheet
import dev.kampr.terminal.view.TerminalView
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val GRID_PANE = "01JKAMPRNODE0000000000000/w3:p1"

private val LINES = listOf(
    "dbrain@comingclean ~/dev/kampr $ cargo test -p kampr-term",
    "test result: ok. 9 passed; 0 failed",
    "dbrain@comingclean ~/dev/kampr $ rm -rf build",
)

private class GridIo(private val conversation: Boolean) : PaneIo {
    val sent = mutableListOf<ClientMsg>()
    var shown: dev.kampr.shared.ui.PaneView? = null
    override fun send(msg: ClientMsg) { sent += msg }
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) = PaneInfo(
        id = GRID_PANE, nodeId = "01JKAMPRNODE0000000000000", workspace = "kampr", tab = "1",
        cwd = "~/dev/kampr", agent = if (conversation) "claude" else null,
        agentStatus = "idle", cols = 62, rows = 24, hasConversation = conversation,
    )
    override fun show(view: dev.kampr.shared.ui.PaneView) { shown = view }
}

private fun gridPane(): PaneState {
    val pane = PaneState(GRID_PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = GRID_PANE,
            cols = 62,
            rows = 24,
            rowsData = LINES.mapIndexed { index, text -> RowDiff(21 + index, listOf(Run(0, text))) },
            cursor = Cursor(LINES.last().length, 23, true),
            links = emptyList(),
        ),
    )
    return pane
}

private fun tokens(): KamprTokens {
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    return KamprTokens(SoftTheme, fonts, typography(fonts, SoftTheme.label, TypeScale.Phone))
}

@Composable
private fun Themed(io: PaneIo, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides tokens(), LocalPaneIo provides io) {
        Box(Modifier.fillMaxSize()) { content() }
    }
}

// ADR 0010. A cell grid does not linearise, so what a screen reader is handed is a description of
// the surface and the one line that is actually moving — not 24 rows of column-padded text.
@OptIn(ExperimentalTestApi::class)
class GridAccessibilityTest {
    @Test
    fun theGridDescribesItselfRatherThanReadingItselfOut() = runComposeUiTest {
        val io = GridIo(conversation = false)
        setContent { Themed(io) { TerminalView(gridPane(), PaneSession(GRID_PANE), io) } }
        onNodeWithContentDescription("Terminal grid", substring = true)
            .assert(SemanticsMatcher.keyIsDefined(SemanticsActions.OnClick))
        onNodeWithContentDescription("62 columns by 24 rows", substring = true).assertExists()
        onNodeWithContentDescription("cursor on row 24, column 46", substring = true).assertExists()
    }

    // The line under the cursor is the unit of speech, and it settles before it speaks so a pane
    // repainting at frame rate cannot talk over itself.
    @Test
    fun theCursorLineIsAPoliteLiveRegion() = runComposeUiTest {
        val io = GridIo(conversation = false)
        setContent { Themed(io) { TerminalView(gridPane(), PaneSession(GRID_PANE), io) } }
        waitUntil(timeoutMillis = 4_000) {
            onAllNodesWithContentDescription("rm -rf build", substring = true)
                .fetchSemanticsNodes().isNotEmpty()
        }
        onNodeWithContentDescription(LINES.last(), substring = true)
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Polite))
    }

    // Where a transcript exists it is the better surface and the grid says so, with a way to get
    // there that does not involve finding a tab by touch.
    @Test
    fun aPaneWithATranscriptPointsAtIt() = runComposeUiTest {
        val io = GridIo(conversation = true)
        setContent { Themed(io) { TerminalView(gridPane(), PaneSession(GRID_PANE), io) } }
        onNodeWithContentDescription("Conversation view of this pane is ordinary text", substring = true)
            .assertExists()
    }

    // The caps were driven by a raw tap detector, which TalkBack's double tap never reaches: they
    // were reachable, unnamed and unpressable at the same time.
    @Test
    fun everyKeyCapIsNamedAndPressableWithoutAFinger() = runComposeUiTest {
        val io = GridIo(conversation = false)
        val session = PaneSession(GRID_PANE)
        val sink = InputSink(GRID_PANE, io, session.latches)
        setContent { Themed(io) { PaneKeyRow(session, sink, compact = false, enabled = true) } }

        for (name in listOf("Escape key", "Control", "Tab key", "Up arrow key", "Slash key", "Page up key")) {
            onNodeWithContentDescription(name).assertExists()
        }
        onNodeWithContentDescription("Slash key")
            .assertHeightIsAtLeast(44.dp)
            .performSemanticsAction(SemanticsActions.OnClick)
        assertTrue(
            io.sent.any { it is ClientMsg.InputText && it.text == "/" },
            "a semantics click on a cap sent nothing: ${io.sent}",
        )
    }

    // The sheet appears because Enter was pressed, not because anyone looked for it, and the
    // command it is about is the whole content of the decision.
    @Test
    fun theDestructiveGuardInterruptsAndReadsOutTheCommand() = runComposeUiTest {
        val io = GridIo(conversation = false)
        val pane = gridPane()
        val session = PaneSession(GRID_PANE)
        val guard = SubmitGuard(pane, io, session.confirm)
        InputSink(GRID_PANE, io, session.latches, guard).raw(Esc.ENTER)
        val held = requireNotNull(session.confirm.held) { "the guard did not hold a real command" }
        assertEquals("dbrain@comingclean ~/dev/kampr $ rm -rf build".substringAfter("$ "), held.command)

        setContent { Themed(io) { ConfirmSheet(held, {}, {}, {}) } }
        onNodeWithContentDescription("Before this runs. ${held.reason}. The command is: ${held.command}")
            .assert(SemanticsMatcher.expectValue(SemanticsProperties.LiveRegion, LiveRegionMode.Assertive))
        onNodeWithContentDescription("Run ${held.command}").assertExists()
        onNodeWithContentDescription("Back to edit — do not run it").assertExists()

        // The guard is the one sheet where a keyboard user must be able to say no without hunting
        // for a control, so it takes focus on open and Escape means "back to edit".
        assertTrue(
            onAllNodes(isFocused()).fetchSemanticsNodes().isNotEmpty(),
            "the confirm sheet opened without taking focus",
        )
    }
}
