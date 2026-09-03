package dev.kampr.terminal

import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.getUnclippedBoundsInRoot
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.runComposeUiTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val BODY = "the whole file\nand its second line"

private fun manyLines(count: Int) = (0 until count).joinToString("\n") { "line $it" }

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.readFile(text: String, desk: Boolean): Pasteboard {
    val session = PaneSession(Phone.PANE)
    val route = Route(words(text))
    val board = if (desk) {
        deskTerminal(paneShowing(SHOWN), session, route)
    } else {
        gridTerminal(paneShowing(SHOWN), session, route)
    }
    tapCell(session, 0, SHOWN.indexOf(NOTES) + 2)
    onNodeWithContentDescription("Open $NOTES").performClick()
    waitUntil(timeoutMillis = 10_000) {
        onAllNodesWithContentDescription("Close $NOTES").fetchSemanticsNodes().isNotEmpty()
    }
    return board
}

// The operator's report was "no close button, i also need to press escape to close". There is one,
// and there always was: `CloseAction` is in the sheet's header row and a keyboard reaches it.
//
// It is *underneath the pane header*. `PaneScreen` paints the terminal surface edge to edge and
// floats its own bar over the top of it — which is why `ZoomSheet` and `ConfirmSheet` are both
// given `chromeTop` — and `FileSheet` was the one surface that laid its controls out at y=0 and
// was covered by an opaque bar for its trouble. Not invisible, not unclear: occluded.
@OptIn(ExperimentalTestApi::class)
class ReadingAFileTheGridOpenedTest {
    @Test
    fun theCloseButtonIsNotUnderneathTheHeaderThatFloatsOverThisSurface() = runComposeUiTest {
        readFile(BODY, desk = true)
        val close = onNodeWithContentDescription("Close $NOTES").getUnclippedBoundsInRoot()
        assertTrue(
            close.top >= CHROME,
            "the close button is at ${close.top}, under the ${CHROME} of pane header painted over it",
        )
    }

    @Test
    fun aPhoneSizedPaneKeepsItsCloseButtonClearOfTheHeaderToo() = runComposeUiTest {
        readFile(BODY, desk = false)
        val close = onNodeWithContentDescription("Close $NOTES").getUnclippedBoundsInRoot()
        assertTrue(close.top >= CHROME, "the close button is at ${close.top}, behind the header")
    }

    // The whole reason the file is worth opening. `KText` draws a `BasicText`, which is inert
    // unless something above it says otherwise — and a `BasicText` that can be selected is the one
    // thing on this screen that claims the I-beam for itself, which is how a test can see it.
    @Test
    fun theWordsInTheViewerCanBeSelected() = runComposeUiTest {
        readFile(BODY, desk = true)
        val body = onNodeWithText(BODY, useUnmergedTree = true).fetchSemanticsNode()
        assertEquals(TEXT, body.cursor(), "the file body hovers as ${body.cursor()}, so it is not selectable")
    }

    @Test
    fun aCopyControlTakesTheWholeFile() = runComposeUiTest {
        val board = readFile(BODY, desk = true)
        onNodeWithContentDescription("Copy $NOTES").performClick()
        assertEquals(BODY, board.held?.text)
    }

    // The layout cap is a cost, not a truncation of the file — so Copy takes all of it and says
    // how many lines that is. A button that quietly stopped at the two thousandth line of a file
    // the operator had just scrolled to the end of would be the expensive kind of plausible.
    @Test
    fun aFileTallerThanTheLayoutCapSaysWhatCopyWillTakeAndThenTakesIt() = runComposeUiTest {
        val whole = manyLines(2_100)
        val board = readFile(whole, desk = true)
        val label = "Copy all 2100 lines of $NOTES, including the ones not shown"
        onNodeWithContentDescription(label).performClick()
        assertEquals(whole, board.held?.text)
    }
}
