package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasSetTextAction
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
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

private const val GRID = 53
private const val PROMPT = "[09:54:25 dbrain@comingclean kob-public]$ "
private val RULE = "─".repeat(120)

private object Held : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs(mapOf("zoom" to "1.0"))
}

private fun rows(written: Map<Int, String>) = (0 until GRID).map { RowDiff(it, written[it]?.let { text -> listOf(Run(0, text)) } ?: emptyList()) }

private fun PaneState.firstFrame() {
    applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = 120,
            rows = GRID,
            rowsData = rows(mapOf(0 to PROMPT)),
            cursor = Cursor(PROMPT.length, 0, true),
            links = emptyList(),
        ),
    )
}

private val claude = mapOf(
    1 to " ▐▛███▜▌   Claude Code v2.1.282",
    2 to "▝▜█████▛▘  Opus 5.5 with medium effort",
    3 to "  ▘▘ ▝▝    ~/dev/kob-public",
    GRID - 4 to RULE,
    GRID - 3 to "❯ ",
    GRID - 2 to RULE,
    GRID - 1 to "  ⏵⏵ bypass permissions on (shift+tab to cycle)",
)

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.settle(ms: Long = 2_000) {
    mainClock.advanceTimeBy(ms)
    waitForIdle()
}

// The operator, on the phone: a new tab, `dangerclaude` typed with the keyboard up, enter — and
// "bypass permissions on" was under the column bar, the composer the last row on the screen.
@OptIn(ExperimentalTestApi::class)
class StartingClaudeOnAPhoneTest {
    @Test
    fun claudeStartedFromAFreshShellShowsTheLinesUnderItsComposer() = runComposeUiTest {
        val pane = PaneState(Phone.PANE, StyleTable())
        val session = PaneSession(Phone.PANE)
        val bars = phoneTerminal(pane, session, width = 485.dp, height = 1083.dp, io = Held)
        pane.firstFrame()
        settle()
        session.openKeyboard()
        bars.value = Phone.KEYBOARD
        settle()
        onNode(hasSetTextAction()).performTextInput("dangerclaude\n")
        pane.applyPatch(
            ServerMsg.GridPatch(pane = Phone.PANE, rows = rows(mapOf(0 to PROMPT + "dangerclaude")), cursor = Cursor(0, 1, true), links = emptyList()),
        )
        settle(900)
        pane.applyPatch(
            ServerMsg.GridPatch(pane = Phone.PANE, rows = rows(claude), cursor = Cursor(2, GRID - 3, true), links = emptyList()),
        )
        settle()
        val state = { "scroll ${session.view.scrollY}, floor ${session.view.band.floor}, max ${session.view.maxScroll}" }
        assertTrue(onScreen(pane, session, GRID - 3), "the composer is off the screen: ${state()}")
        assertTrue(onScreen(pane, session, GRID - 1), "the mode line is off the screen: ${state()}")
    }
}
