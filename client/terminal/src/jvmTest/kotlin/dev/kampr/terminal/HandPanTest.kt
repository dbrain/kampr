package dev.kampr.terminal

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertTrue

private const val PANE = Phone.PANE

// A pane wider than any phone, with the caret parked near column 0 where every agent's input box
// puts it — and near the *bottom* of the grid, because follow-cursor only runs on a surface that
// is bottom-pinned, which is what an agent pane with its prompt at the foot of the screen is.
// Reading the right-hand end of a line like this is the whole reason the surface pans at all.
private fun wideShell(): PaneState {
    val pane = PaneState(PANE, StyleTable())
    val long = (0 until 20).joinToString(" ") { "column-group-$it" }
    pane.applyReset(
        ServerMsg.GridReset(
            pane = PANE,
            cols = 200,
            rows = 40,
            rowsData = listOf(RowDiff(0, listOf(Run(0, long))), RowDiff(38, listOf(Run(0, "> ")))),
            cursor = Cursor(2, 38, true),
            links = emptyList(),
        ),
    )
    return pane
}

private class Harnessed(val agent: String?) : PaneIo {
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String) = PanePrefs()
    override fun info(paneId: String) = PaneInfo(id = paneId, nodeId = "node", agent = agent)
}

private fun PaneState.caretTo(col: Int, visible: Boolean = true) = applyPatch(
    ServerMsg.GridPatch(
        pane = PANE,
        rows = emptyList(),
        cursor = Cursor(col, 38, visible),
        links = emptyList(),
    ),
)

@OptIn(ExperimentalTestApi::class)
class HandPanTest {
    // The report, verbatim: "drag with finger, hit edge of screen, move and drag some more, scroll
    // resets from start of line rather than where it was, so if i need to scroll more than 1
    // finger movement it resets".
    //
    // Follow-cursor was armed for the life of the pane. `followCursorPan` puts the caret a margin
    // in from the left, and a caret inside that margin puts the surface at exactly panX = 0 — the
    // start of the line, in the operator's words. So every frame that moved the caret undid the
    // pan, and a second drag started over.
    @Test
    fun aPanTheHandMadeSurvivesTheCaretMoving() = runComposeUiTest {
        val pane = wideShell()
        val session = PaneSession(PANE)
        phoneTerminal(pane, session)
        assertTrue(session.view.minPanX < -1f, "the grid has to overflow, or nothing is tested")

        session.view.scrollBy(session.view.minPanX, 0f)
        waitForIdle()
        val reached = session.view.panX
        assertTrue(reached < -1f, "the drag went nowhere: panX is $reached")

        pane.caretTo(3)
        waitForIdle()
        assertTrue(
            session.view.panX == reached,
            "the caret moved and took the surface back to ${session.view.panX} of $reached",
        )
    }

    // And the other half: follow-cursor is not switched off for good, or a long line would stop
    // carrying the caret the moment anyone had ever dragged. The hand owns the axis only until the
    // caret comes back onto the screen by itself.
    @Test
    fun theCaretTakesTheAxisBackWhenItReturnsToTheScreen() = runComposeUiTest {
        val pane = wideShell()
        val session = PaneSession(PANE)
        phoneTerminal(pane, session)
        val cell = session.grid.cellWidth
        assertTrue(cell > 1f, "the grid never laid out, so nothing was measured")

        session.view.scrollBy(-30f * cell, 0f)
        waitForIdle()
        val reached = session.view.panX

        // Still on screen at this pan, so the surface stays where the hand left it and re-arms.
        pane.caretTo(34)
        waitForIdle()
        assertTrue(session.view.panX == reached, "a caret already in view moved the surface")

        // Now it runs off the right-hand edge, and following it is the point of the feature.
        pane.caretTo(150)
        waitForIdle()
        assertTrue(
            session.view.panX < reached,
            "the caret ran off the edge and the surface did not follow: ${session.view.panX}",
        )
    }

    // The operator, on a phone: *"it scrolled right a bunch opening top when I wasn't scrolled on
    // previous screen so I couldn't see anything without scrolling left again"*.
    //
    // **A caret the program has hidden is not where anybody is typing.** Probe #499: `top` sets
    // `?25l` and parks the cursor at **column 92 of a 94-column pane** for its whole run — nothing
    // is drawn there and nothing can be, because the cursor is not on the screen. The surface
    // chased it anyway and took the pane two screen-widths to the right, on a pane nobody had
    // panned; every full-screen program does the same thing, since hiding the cursor is what you
    // do before painting a frame you own.
    @Test
    fun a_caret_the_program_has_hidden_does_not_drag_the_surface_across_the_pane() = runComposeUiTest {
        val pane = wideShell()
        val session = PaneSession(PANE)
        phoneTerminal(pane, session)
        assertTrue(session.view.minPanX < -1f, "the grid has to overflow, or nothing is tested")
        assertTrue(session.view.panX == 0f, "the surface opens at the start of the line")

        pane.caretTo(150, visible = false)
        waitForIdle()

        assertTrue(
            session.view.panX == 0f,
            "a full-screen program parked a caret nobody can see at column 150 and the surface " +
                "followed it to ${session.view.panX}",
        )
    }

    // **pi hides its caret for its whole run and parks it where the next character goes** —
    // measured typing a line longer than the pane (research/probe/pi-typing.py): `?25l` and never
    // `?25h`, the caret at column 20, 40, 60, 80 behind the text, where Claude shows its own. So the
    // hidden-caret rule above, which is right for `top`, left every character typed into pi
    // wherever the surface happened to be. The harness is what tells the two apart: typing `top`
    // is a keystroke too, so "hidden, but moved after a send" does not.
    @Test
    fun a_hidden_caret_is_followed_on_a_harness_that_types_behind_it() = runComposeUiTest {
        for ((agent, follows) in listOf("pi" to true, null to false)) {
            val pane = wideShell()
            val session = PaneSession(PANE)
            phoneTerminal(pane, session, io = Harnessed(agent))
            assertTrue(session.view.minPanX < -1f, "the grid has to overflow, or nothing is tested")

            pane.caretTo(150, visible = false)
            waitForIdle()

            assertTrue(
                (session.view.panX < -1f) == follows,
                "agent $agent: a hidden caret at column 150 left the surface at ${session.view.panX}",
            )
        }
    }

    // The report: typing a long line in pi, the surface does not follow the caret to the right,
    // and once the operator has scrolled to see the end of it, every further keystroke that runs
    // the caret off the edge is not followed either — the manual scroll owns the axis for good.
    // A keystroke is what hands it back: `followAgain` clears `pannedAway`, so the next frame that
    // moves the caret off the edge answers the request.
    @Test
    fun aKeystrokeReArmsTheHorizontalFollowAfterAManualScroll() = runComposeUiTest {
        val pane = wideShell()
        val session = PaneSession(PANE)
        phoneTerminal(pane, session)
        assertTrue(session.view.minPanX < -1f, "the grid has to overflow, or nothing is tested")
        val cell = session.grid.cellWidth

        // A partial pan: the hand takes the axis, but not all the way to the left, so a follow
        // that answers a later keystroke has room to move the surface further.
        session.view.scrollBy(-20f * cell, 0f)
        waitForIdle()
        val reached = session.view.panX
        assertTrue(reached < -1f, "the drag went nowhere: panX is $reached")

        // The hand owns the axis: a caret that runs off the edge is not chased.
        pane.caretTo(150)
        waitForIdle()
        assertTrue(session.view.panX == reached, "a panned axis chased a caret off screen")

        // A keystroke re-arms it, and the caret that runs off the edge is followed again.
        session.view.followAgain()
        pane.caretTo(180)
        waitForIdle()
        assertTrue(
            session.view.panX < reached,
            "a keystroke re-armed the follow and the surface did not answer it: ${session.view.panX}",
        )
    }
}
