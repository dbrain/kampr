package dev.kampr.terminal

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.filePlace
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// W6: `FileRef` — the whole node half of file retrieval — was complete, tested, and minted by
// nothing in `client/`. The conversation surface mints one now; the terminal grid is where an
// operator watching `cargo` or `claude` actually reads a path, and it minted nothing.
@OptIn(ExperimentalTestApi::class)
class OpeningAFileFromTheGridTest {
    @Test
    fun tappingAPathOnTheGridOffersToOpenTheFileItNames() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        gridTerminal(paneShowing(SHOWN), session, Route(words("hello")))
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        onNodeWithContentDescription("Open $NOTES").assertExists()
    }

    // The grid fetches nothing itself: the path goes onto the app's files stack, whose viewer
    // fetches it, covers the window, and is what the phone's back button closes first.
    @Test
    fun openingItHandsThatPathOnThisPanesNodeToTheFileViewer() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        val route = Route(words("the whole file\nand its second line"))
        val opened = mutableListOf<FilesPlace>()
        gridTerminal(paneShowing(SHOWN), session, route, opened = opened)
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        onNodeWithContentDescription("Open $NOTES").performClick()
        waitForIdle()
        assertEquals(listOf<FilesPlace>(filePlace(Phone.PANE, NOTES)), opened)
        assertEquals(Phone.PANE.substringBefore('/'), opened.single().nodeId)
        assertTrue(route.asked.isEmpty(), "the grid fetched the file as well as handing it over")
        assertTrue(
            onAllNodesWithContentDescription("Open $NOTES").fetchSemanticsNodes().isEmpty(),
            "the strip offering the path stayed up after it was opened",
        )
    }

    // The route is gated on a device that may send input, and the whole security argument for this
    // form of id is that such a device can already `cat` the file. A device that may not type is
    // exactly the one that must not reach `~/.ssh/id_rsa`, so it is offered the string instead.
    @Test
    fun aReadOnlyDeviceIsOfferedTheStringRatherThanTheFile() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        val route = Route(words("secrets"), readOnly = true)
        gridTerminal(paneShowing(SHOWN), session, route)
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        onNodeWithContentDescription("Copy $NOTES").assertExists()
        assertTrue(
            onAllNodesWithContentDescription("Open $NOTES").fetchSemanticsNodes().isEmpty(),
            "a device that cannot type was offered a way to read a file off the host",
        )
        assertTrue(route.asked.isEmpty(), "the route was asked anyway")
    }
}
