package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performMouseInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.terminal.render.GridPoint
import dev.kampr.terminal.render.Selection
import kotlin.test.Test
import kotlin.test.assertNull

// The operator, on the desk in Chrome: *"no way to close the selection without clicking
// copy/paste (clicking away used to work)"*, and a click on a link offered nothing. A click's
// release never ended the gesture there, so the click was never a tap.
@OptIn(ExperimentalTestApi::class)
class BrowserClickTest {
    @Test
    fun aClickPutsTheSelectionAway() = runComposeUiTest {
        val session = PaneSession(BROWSER_PANE)
        browserTerminal(writtenPane(rows = 30, caretRow = 29), session, DESK)
        session.view.selection = Selection(GridPoint(10, 0), GridPoint(10, 12))
        frames(2)

        onRoot().performMouseInput {
            moveTo(Offset(width * 0.7f, height * 0.6f))
            press()
            release()
        }
        frames(4)

        assertNull(session.view.selection, "a click left the selection standing")
    }
}
