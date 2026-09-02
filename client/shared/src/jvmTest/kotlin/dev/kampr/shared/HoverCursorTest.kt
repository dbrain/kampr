package dev.kampr.shared

import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.semantics.getOrNull
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.hasClickAction
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.runComposeUiTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotEquals
import kotlin.test.assertTrue

// The operator, verbatim: "the buttons on hover have a selection cursor". Every chip in this sheet
// paints its caption with `BasicText`, the sheet mounts its body in a `SelectionContainer` so the
// caption is selectable, and a selectable `BasicText` asks for the I-beam — so a row of buttons
// hovered as prose. Nothing in this client had ever asked for a cursor at all.
@OptIn(ExperimentalTestApi::class)
class HoverCursorTest {
    @Test
    fun everyCaptionedControlInTheActionsSheetHoversAsAHand() = runComposeUiTest {
        setContent { Actions() }
        val controls = onAllNodes(hasClickAction(), useUnmergedTree = true)
            .fetchSemanticsNodes()
            .filter { it.paintsText() }
        assertTrue(controls.size >= 8, "the sheet painted ${controls.size} captioned controls")
        for (control in controls) {
            val name = control.config.getOrNull(SemanticsProperties.ContentDescription)?.joinToString()
            assertEquals(HAND, control.cursor(), "\"$name\" hovers as ${control.cursor()}")
        }
    }

    @Test
    fun aSheetTellsItsControlsFromItsProse() = runComposeUiTest {
        setContent { Mixed() }
        assertEquals(HAND, onNodeWithContentDescription("Close Details").fetchSemanticsNode().cursor())
        assertEquals(HAND, onNodeWithContentDescription("rename").fetchSemanticsNode().cursor())
        assertEquals(TEXT, onNodeWithText(SHEET_PROSE).fetchSemanticsNode().cursor())

        val refused = onNodeWithContentDescription("Send").fetchSemanticsNode()
        assertEquals(ARROW, refused.cursor(), "a control that refuses the press is not a hand")

        val card = onNodeWithContentDescription("front, this machine").fetchSemanticsNode()
        assertNotEquals(HAND, card.cursor(), "a named row that is not a control is not a hand")
        assertTrue(card.layoutInfo.getModifierInfo().none { HOVER.containsMatchIn(it.modifier.toString()) })
    }
}
