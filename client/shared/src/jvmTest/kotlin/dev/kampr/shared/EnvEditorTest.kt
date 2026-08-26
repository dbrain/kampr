package dev.kampr.shared

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.onAllNodesWithContentDescription
import androidx.compose.ui.test.onNodeWithContentDescription
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performTextInput
import androidx.compose.ui.test.performTextInputSelection
import androidx.compose.ui.test.runComposeUiTest
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.unit.dp
import dev.kampr.shared.ui.EnvEditor
import kotlin.test.Test
import kotlin.test.assertEquals

private const val NAME = "Variable name"

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.rows(vararg initial: Pair<String, String>): MutableList<Pair<String, String>> {
    val rows = mutableStateListOf(*initial)
    setContent {
        Bars {
            Box(Modifier.size(420.dp, 900.dp)) {
                EnvEditor(rows, { index, row -> rows[index] = row }, { rows.add("" to "") }, { rows.removeAt(it) })
            }
        }
    }
    waitForIdle()
    return rows
}

@OptIn(ExperimentalTestApi::class)
private fun ComposeUiTest.nameField(index: Int): String =
    onAllNodesWithContentDescription(NAME)[index].fetchSemanticsNode()
        .config[SemanticsProperties.EditableText].text

// A field handed a TextFieldValue built fresh on every recomposition is handed a caret at offset
// zero with it, so every character lands in front of the one before it and PATH is typed HTAP.
@OptIn(ExperimentalTestApi::class)
class EnvEditorTest {
    @Test
    fun a_variable_name_comes_out_in_the_order_it_was_typed() = runComposeUiTest {
        val rows = rows("" to "")
        for (ch in "PATH") onNodeWithContentDescription(NAME).performTextInput(ch.toString())
        assertEquals("PATH", nameField(0), "the caret went back to the start between keystrokes")
        assertEquals("PATH", rows[0].first)
    }

    @Test
    fun a_variable_value_comes_out_in_the_order_it_was_typed() = runComposeUiTest {
        val rows = rows("PATH" to "")
        for (ch in "/usr/bin") onNodeWithContentDescription("Value of PATH").performTextInput(ch.toString())
        assertEquals("/usr/bin", rows[0].second)
    }

    // The rows have no identity beyond their position, so a field that keeps its own caret has to
    // notice when the row underneath it becomes a different row.
    @Test
    fun removing_a_variable_leaves_the_row_below_it_showing_its_own_name() = runComposeUiTest {
        val rows = rows("ONE" to "1", "TWO" to "2")
        onNodeWithContentDescription("Remove ONE").performClick()
        waitForIdle()
        assertEquals(listOf("TWO" to "2"), rows.toList())
        assertEquals("TWO", nameField(0), "the field kept the removed row's text")
    }

    // Not merely "the caret is not at the start": a field that re-seeded itself to the end of the
    // text on every recomposition would type PATH correctly and still refuse to be edited in the
    // middle of a name already there.
    @Test
    fun a_variable_name_can_be_corrected_in_the_middle() = runComposeUiTest {
        val rows = rows("PTH" to "")
        onNodeWithContentDescription(NAME).performTextInputSelection(TextRange(1))
        onNodeWithContentDescription(NAME).performTextInput("A")
        assertEquals("PATH", rows[0].first)
    }
}
