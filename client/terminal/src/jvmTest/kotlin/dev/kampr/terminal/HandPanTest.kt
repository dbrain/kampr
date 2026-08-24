package dev.kampr.terminal

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
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

private fun PaneState.caretTo(col: Int) = applyPatch(
    ServerMsg.GridPatch(pane = PANE, rows = emptyList(), cursor = Cursor(col, 38, true), links = emptyList()),
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
}
