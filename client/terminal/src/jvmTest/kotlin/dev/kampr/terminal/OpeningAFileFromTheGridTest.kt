package dev.kampr.terminal

import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
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

    @Test
    fun openingItAsksTheRouteForThatPathAndShowsWhatCameBack() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        val route = Route(words("the whole file\nand its second line"))
        gridTerminal(paneShowing(SHOWN), session, route)
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        onNodeWithContentDescription("Open $NOTES").performClick()
        waitUntil(timeoutMillis = 5_000) { route.asked.isNotEmpty() }
        assertEquals(listOf(fileAttachmentId(NOTES)), route.asked)
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithContentDescription("Close $NOTES").fetchSemanticsNodes().isNotEmpty()
        }
        onNodeWithText("the whole file\nand its second line", substring = true).assertExists()
    }

    // #233's lesson: a refusal that reads as a plausible success is the expensive kind. The route
    // gives one uniform 404 for missing, unreadable and escaped, and that is what has to be shown.
    @Test
    fun aRouteThatRefusesSaysSoRatherThanShowingAnEmptyFile() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        val route = Route(AttachmentBytes.Failed("The node no longer has this attachment."))
        gridTerminal(paneShowing(SHOWN), session, route)
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        onNodeWithContentDescription("Open $NOTES").performClick()
        waitUntil(timeoutMillis = 5_000) {
            onAllNodesWithText("The node no longer has this attachment.").fetchSemanticsNodes().isNotEmpty()
        }
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
