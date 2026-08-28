package dev.kampr.terminal

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onAllNodesWithText
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTouchInput
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val NOTES = "/home/dbrain/dev/kampr/notes.md"
private const val SHOWN = "$ cat $NOTES"

private class Route(
    private val answer: AttachmentBytes,
    override val readOnly: Boolean = false,
) : PaneIo {
    val asked = mutableListOf<String>()
    override fun send(msg: ClientMsg) = Unit
    override fun prefs(paneId: String): PanePrefs = PanePrefs()
    override suspend fun attachment(paneId: String, id: String): AttachmentBytes {
        asked += id
        return answer
    }
}

private fun words(text: String) = AttachmentBytes.Ok(text.encodeToByteArray(), "text/plain")

// As tall as the desktop that made it, with the caret on the last line written. Both matter to
// where a tap lands. The default zoom is max(fit-width, fit-height), so an eight-row pane on a
// phone is blown up until barely nine columns are on screen and a column eight cells in is off
// the right edge — and how far off depends on the line height of whatever font the machine
// resolves for monospace, which is why that only ever failed on a runner. A full-height pane is
// width-fit instead, and the caret's row is the one row `caretFloor` guarantees is on screen.
private fun paneShowing(vararg lines: String): PaneState {
    val pane = PaneState(Phone.PANE, StyleTable())
    pane.applyReset(
        ServerMsg.GridReset(
            pane = Phone.PANE,
            cols = 94,
            rows = 40,
            rowsData = lines.mapIndexed { row, text -> RowDiff(row, listOf(Run(0, text))) },
            cursor = Cursor(lines.last().length, lines.lastIndex, true),
            links = emptyList(),
        ),
    )
    return pane
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.tapCell(session: PaneSession, row: Int, col: Int) {
    val grid = session.grid
    assertTrue(grid.cellWidth > 1f, "the grid has not been painted, so nothing is being tapped")
    val at = Offset(
        grid.originX + (col + 0.5f) * grid.cellWidth,
        grid.originY + (row + 0.5f) * grid.cellHeight,
    )
    // A point off the surface is delivered to nothing at all, so without this the test reads as
    // "the grid does not offer paths" when what happened is that the cell was never touched.
    val surface = onRoot().fetchSemanticsNode().size
    assertTrue(
        at.x >= 0f && at.y >= 0f && at.x < surface.width && at.y < surface.height,
        "cell $row,$col is painted at $at, outside the ${surface.width}x${surface.height} surface",
    )
    onRoot().performTouchInput {
        down(at)
        up()
    }
    waitForIdle()
}

// W6: `FileRef` — the whole node half of file retrieval — was complete, tested, and minted by
// nothing in `client/`. The conversation surface mints one now; the terminal grid is where an
// operator watching `cargo` or `claude` actually reads a path, and it minted nothing.
@OptIn(ExperimentalTestApi::class)
class OpeningAFileFromTheGridTest {
    @Test
    fun tappingAPathOnTheGridOffersToOpenTheFileItNames() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        phoneTerminal(paneShowing(SHOWN), session, io = Route(words("hello")))
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        onNodeWithContentDescription("Open $NOTES").assertExists()
    }

    @Test
    fun openingItAsksTheRouteForThatPathAndShowsWhatCameBack() = runComposeUiTest {
        val session = PaneSession(Phone.PANE)
        val route = Route(words("the whole file\nand its second line"))
        phoneTerminal(paneShowing(SHOWN), session, io = route)
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
        phoneTerminal(paneShowing(SHOWN), session, io = route)
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
        phoneTerminal(paneShowing(SHOWN), session, io = route)
        tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
        onNodeWithContentDescription("Copy $NOTES").assertExists()
        assertTrue(
            onAllNodesWithContentDescription("Open $NOTES").fetchSemanticsNodes().isEmpty(),
            "a device that cannot type was offered a way to read a file off the host",
        )
        assertTrue(route.asked.isEmpty(), "the route was asked anyway")
    }
}
