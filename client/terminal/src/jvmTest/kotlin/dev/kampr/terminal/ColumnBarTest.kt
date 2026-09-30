package dev.kampr.terminal

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasContentDescription
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.unit.dp
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val columnBar = hasContentDescription("Showing columns", substring = true)

@OptIn(ExperimentalTestApi::class)
class ColumnBarTest {
    @Test
    fun aPaneThatFitsItsViewKeepsNoBarUnderIt() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session, width = 1280.dp, height = 800.dp)
        assertTrue(
            onAllNodes(columnBar).fetchSemanticsNodes().isEmpty(),
            "every column and the live edge are on screen, so there is no position to report",
        )
        assertEquals(0f, session.indicatorHeight, "the grid is still reserving room for a bar that says nothing")
    }

    @Test
    fun aPaneWiderThanItsViewSaysWhichColumnsAreShowing() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(Phone.shell(), session, io = ReadableIo)
        assertEquals(1, onAllNodes(columnBar).fetchSemanticsNodes().size, "columns are off screen and nothing says so")
        assertEquals(
            0f,
            session.indicatorHeight,
            "the bar took room from the grid, so its coming and going reshapes the view it reports on",
        )
    }
}
