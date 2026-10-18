package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertTrue

private const val GRID = 40
private const val TYPED = "please look at the selection tracking"
private val RULE = "─".repeat(119)

private object Readable : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs(mapOf("zoom" to "1.2"))
}

private fun claudeRows(composer: String): List<RowDiff> {
    val written = mapOf(
        1 to " ▐▛███▛█   Claude Code v2.1.282",
        2 to "▝▜██████▀  Opus 5.5 with medium effort",
        3 to " ▝▝   ▝▝   ~/dev/kampr",
        GRID - 4 to RULE,
        GRID - 3 to "❯ $composer",
        GRID - 2 to RULE,
        GRID - 1 to "  ⏵⏵ auto mode on (shift+tab to cycle)",
    )
    return (0 until GRID).map { RowDiff(it, written[it]?.let { text -> listOf(Run(0, text)) } ?: emptyList()) }
}

private fun freshClaude(): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = 120,
            rows = GRID,
            rowsData = claudeRows(""),
            cursor = Cursor(2, GRID - 3, false),
            links = emptyList(),
        ),
    )
    return pane
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.settle(ms: Long = 2_000) {
    mainClock.advanceTimeBy(ms)
    waitForIdle()
}

// The operator, on the phone with a fresh claude: *"I pasted by long pressing on the Claude text
// input near the end of the screen on the right after typing some"* — the pill's Paste was drawn
// off the screen, and the mode line under the composer was under the visible area.
@OptIn(ExperimentalTestApi::class)
class PastingIntoClaudeOnAPhoneTest {
    @Test
    fun aLongPressOnTheComposerOffersAPasteOnTheScreenWithTheModeLineInView() = runComposeUiTest {
        val pane = freshClaude()
        val session = PaneSession(Phone.PANE)
        val bars = phoneTerminal(pane, session, io = Readable, clipboard = "pasted")
        settle()
        session.openKeyboard()
        bars.value = Phone.KEYBOARD
        settle()
        onNode(hasSetTextAction()).performTextInput(TYPED)
        pane.applyPatch(
            ServerMsg.GridPatch(pane = Phone.PANE, rows = claudeRows(TYPED), cursor = Cursor(2 + TYPED.length, GRID - 3, false), links = emptyList()),
        )
        settle()
        val state = { "scroll ${session.view.scrollY}, floor ${session.view.band.floor}, max ${session.view.maxScroll}" }
        assertTrue(onScreen(pane, session, GRID - 1), "after typing, the mode line is off the screen: ${state()}")

        val y = with(density) { rowTop(pane, session, GRID - 3).toPx() } + session.grid.cellHeight / 2f
        val x = onRoot().getUnclippedBoundsInRoot().right.value * density.density * 0.85f
        onNodeWithContentDescription("Terminal grid", substring = true).performTouchInput {
            down(Offset(x, y))
            advanceEventTime(900)
            moveTo(Offset(x, y))
            up()
        }
        settle(500)
        assertTrue(session.view.selection != null, "the long press selected nothing")
        val paste = onNodeWithContentDescription("Paste the clipboard into the pane").getUnclippedBoundsInRoot()
        assertTrue(
            paste.bottom <= visibleBottom() && paste.top >= Phone.HEADER,
            "Paste is drawn at ${paste.top}..${paste.bottom}, outside ${Phone.HEADER}..${visibleBottom()}: ${state()}",
        )
        assertTrue(onScreen(pane, session, GRID - 1), "after the long press, the mode line is off the screen: ${state()}")
    }
}
