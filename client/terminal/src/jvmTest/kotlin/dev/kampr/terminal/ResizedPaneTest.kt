package dev.kampr.terminal

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test

// `pane.size` is the one op that reshapes a pane, and the node answers it by streaming the pane
// again at the width it was given. That arrives as a `grid.reset` at a width the surface has never
// seen, in the middle of a pane the operator is already looking at and about to type into — which
// is a different case from the first reset a pane ever gets, and the one that decides whether the
// prompt wraps where the caret is.
//
// `CellBuffer.cols` is a plain field rather than a Compose state, so nothing about a wider grid
// recomposes this surface by itself: what carries it is the reset's own `revision`. That coupling
// is invisible at the call site and is what this pins.
@OptIn(ExperimentalTestApi::class)
class ResizedPaneTest {
    @Test
    fun aPaneResizedUnderTheOperatorIsLaidOutAtTheWidthItsNextResetCarries() = runComposeUiTest {
        val pane = Phone.shell()
        phoneTerminal(pane, PaneSession(Phone.PANE))
        onNodeWithContentDescription("94 columns by", substring = true).assertExists()

        val line = "resized while the operator was looking at it"
        pane.applyReset(
            ServerMsg.GridReset(
                pane = Phone.PANE,
                cols = 118,
                rows = 40,
                rowsData = listOf(RowDiff(0, listOf(Run(0, line)))),
                cursor = Cursor(line.length, 0, true),
                links = emptyList(),
            ),
        )
        waitForIdle()

        onNodeWithContentDescription("118 columns by", substring = true).assertExists()
    }
}
