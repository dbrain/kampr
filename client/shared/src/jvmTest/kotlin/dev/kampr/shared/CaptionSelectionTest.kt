package dev.kampr.shared

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.shared.model.paneTitle
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// A sheet mounts its body in a `SelectionContainer` on purpose — the pane's path, a node's name and
// a node's own refusal are the strings worth quoting in a report, and a sheet is where they are
// read. What that container must not carry off is the word painted on a button: dragging across
// the pane actions sheet to copy the path spliced "rename" and "close" into the paste, because
// only `PrimaryAction` and `QuietAction` had ever said their captions were chrome.
//
// The line is per element and it is not "is this clickable": a `SheetCard` is clickable and every
// word on it is content.
@OptIn(ExperimentalTestApi::class)
class CaptionSelectionTest {
    @Test
    fun noControlInTheActionsSheetPutsItsOwnCaptionOnTheClipboard() = runComposeUiTest {
        setContent { Actions() }
        val controls = onAllNodes(hasClickAction(), useUnmergedTree = true)
            .fetchSemanticsNodes()
            .filter { it.paintsText() }
        assertTrue(controls.size >= 8, "the sheet painted ${controls.size} captioned controls")
        for (control in controls) {
            val name = control.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString()
            for ((text, run) in control.textRuns()) {
                assertFalse(run.selectsItsOwnText(), "\"$name\" hands \"$text\" to a drag across the sheet")
            }
        }
    }

    // And the half that must survive the fix. A sheet whose real content stopped being copyable to
    // keep a caption out of the paste is a worse sheet than the one this started as: the header
    // names the pane, and that name is the thing an operator drags across a sheet to quote.
    @Test
    fun theActionsSheetStillHandsOverTheNameOfTheThingItIsAbout() = runComposeUiTest {
        setContent { Actions() }
        assertTrue(
            onNodeWithText(paneTitle(SHEET_PANE)).fetchSemanticsNode().selectsItsOwnText(),
            "the sheet header stopped naming its pane copyably",
        )
    }

    // The two halves side by side, in one sheet: a chip and a refused button against a line of
    // prose and a card. A card's subtitle is a path, a node name or an error — content, and the
    // reason this rule is decided per element rather than by asking whether a node is clickable.
    @Test
    fun aSheetTellsACaptionFromTheContentAroundIt() = runComposeUiTest {
        setContent { Mixed() }
        assertTrue(onNodeWithText(SHEET_PROSE).fetchSemanticsNode().selectsItsOwnText(), "the prose")
        assertTrue(onNodeWithText("front").fetchSemanticsNode().selectsItsOwnText(), "a card's title")
        assertTrue(onNodeWithText("this machine").fetchSemanticsNode().selectsItsOwnText(), "a card's subtitle")

        for (caption in listOf("rename", "Send")) {
            val control = onNodeWithContentDescription(caption).fetchSemanticsNode()
            for ((text, run) in control.textRuns()) {
                assertFalse(run.selectsItsOwnText(), "the caption \"$text\" is selectable")
            }
        }
    }
}
