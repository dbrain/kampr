package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performSemanticsAction
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

// `claude` started in a directory it has never been trusted in, measured on a real 2.1.282 through
// research/probe/claude-start.py: the trust prompt at the top of the grid, the caret on its
// `❯ No, exit` row and the footer three rows under it; half a second of a blank grid once it is
// answered; then the session drawn anchored to the *bottom* of the grid — the composer three rows
// from the end, and a rule and the mode line under it. The operator's report: *"the pane seems to
// lose the bottom of the claude screen ... switching away and back seems to fix it"*.
@OptIn(ExperimentalTestApi::class)
class ClaudeStartingInANewDirectoryTest {
    private class Recording : PaneIo {
        val sent = mutableListOf<ClientMsg>()
        override fun send(msg: ClientMsg) {
            sent += msg
        }
        override fun prefs(paneId: String) = PanePrefs(mapOf("zoom" to "1.2"))
    }

    private fun rows(vararg at: Pair<Int, String>): List<RowDiff> {
        val written = at.toMap()
        return (0 until GRID).map { row -> RowDiff(row, written[row]?.let { listOf(Run(0, it)) } ?: emptyList()) }
    }

    private fun PaneState.draw(rows: List<RowDiff>, caret: Cursor) =
        applyPatch(ServerMsg.GridPatch(pane = Phone.PANE, rows = rows, cursor = caret, links = emptyList()))

    private fun ComposeUiTest.settle(ms: Long) {
        mainClock.advanceTimeBy(ms)
        waitForIdle()
    }

    private fun ComposeUiTest.paste() {
        onNodeWithContentDescription("Terminal grid", substring = true).performTouchInput {
            val at = Offset(width * 0.5f, center.y)
            down(at)
            advanceEventTime(900)
            moveTo(at)
            up()
        }
        waitForIdle()
        onNodeWithContentDescription("Paste the clipboard into the pane")
            .performSemanticsAction(SemanticsActions.OnClick)
        waitForIdle()
    }

    private fun ComposeUiTest.atTheTrustPrompt(io: Recording): Pair<PaneState, PaneSession> {
        val pane = PaneState(Phone.PANE, StyleTable())
        pane.applyReset(
            ServerMsg.GridReset(
                pane = Phone.PANE, cols = 120, rows = GRID,
                rowsData = rows(0 to "[09:05:31 dbrain@comingclean kampr-cstart]$ claude"),
                cursor = Cursor(0, 1, true), links = emptyList(),
            ),
        )
        val session = PaneSession(Phone.PANE)
        phoneTerminal(pane, session, io = io, clipboard = "\r")
        settle(2_000)
        pane.draw(
            rows(
                0 to "[09:05:31 dbrain@comingclean kampr-cstart]$ claude",
                2 to RULE,
                3 to " Accessing workspace:",
                5 to " /var/tmp/kampr-cstart",
                7 to " Quick safety check: Is this a project you created or one you trust?",
                10 to " Claude Code'll be able to read, edit, and execute files here.",
                12 to " Security guide",
                14 to " ❯ No, exit",
                15 to "   Yes, I trust this folder",
                17 to " Enter to confirm · Esc to cancel",
            ),
            Cursor(1, 14, false),
        )
        settle(2_000)
        return pane to session
    }

    private fun ComposeUiTest.claudeDrawsItsSession(pane: PaneState) {
        pane.draw(rows(0 to "[09:05:31 dbrain@comingclean kampr-cstart]$ claude"), Cursor(0, 1, false))
        settle(540)
        pane.draw(
            rows(
                1 to " ▐▛███▛█   Claude Code v2.1.282",
                2 to "▝▜██████▀  Opus 5.5 with medium effort",
                3 to " ▝▝   ▝▝   /var/tmp/kampr-cstart",
                GRID - 4 to RULE,
                GRID - 3 to "❯ Try \"edit <filepath> to...\"",
                GRID - 2 to RULE,
                GRID - 1 to "  ⏵⏵ auto mode on (shift+tab to cycle)",
            ),
            Cursor(2, GRID - 3, false),
        )
        settle(2_000)
    }

    private fun ComposeUiTest.theModeLineIsOnScreen(answeredHere: Boolean) {
        val io = Recording()
        val (pane, session) = atTheTrustPrompt(io)
        assertTrue(session.view.maxScroll > 0f, "the grid has to overflow the phone, or nothing is tested")

        if (answeredHere) {
            paste()
            assertTrue(io.sent.isNotEmpty(), "the operator's Enter never reached the pane")
        }
        claudeDrawsItsSession(pane)

        assertTrue(
            onScreen(pane, session, GRID - 1),
            "answered here: $answeredHere — the mode line under the composer is off the bottom of " +
                "the screen: the surface is at ${session.view.scrollY} with the floor at " +
                "${session.view.band.floor}",
        )
    }

    // The Enter pressed on the phone arms a reanchor, and the floor drops in two steps — the
    // record's end at once, the caret's 200 ms later — so a one-shot spent on the first step left
    // the surface resting on the second one's ceiling.
    @Test
    fun theModeLineIsOnScreenOnceThePhoneHasAnsweredTheTrustPrompt() = runComposeUiTest {
        theModeLineIsOnScreen(answeredHere = true)
    }

    // Answered at the desk, or on another device: no byte from this client at all.
    @Test
    fun theModeLineIsOnScreenWhenTheTrustPromptWasAnsweredElsewhere() = runComposeUiTest {
        theModeLineIsOnScreen(answeredHere = false)
    }

    private companion object {
        const val GRID = 40
        val RULE = "─".repeat(119)
    }
}
