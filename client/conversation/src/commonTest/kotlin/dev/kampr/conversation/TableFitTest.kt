package dev.kampr.conversation

import dev.kampr.conversation.md.fitColumns
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class TableFitTest {
    @Test
    fun aNarrowTableGrowsToFillThePaneExactly() {
        val fitted = fitColumns(listOf(100f, 60f, 40f), 400f)
        assertEquals(400f, fitted.sum())
        assertTrue(fitted[0] > fitted[1] && fitted[1] > fitted[2])
    }

    // Shrinking a wide table to fit is what turns it back into mush. It keeps its natural widths
    // and the overflow goes to its own scroller, which is why the page never moves sideways.
    @Test
    fun aWideTableKeepsItsNaturalWidthsAndOverflows() {
        val natural = listOf(60f, 460f, 460f, 460f)
        val fitted = fitColumns(natural, 358f)
        assertEquals(natural, fitted)
        assertTrue(fitted.sum() > 358f)
    }

    @Test
    fun anEmptyTableDoesNotDivideByZero() {
        assertEquals(emptyList(), fitColumns(emptyList(), 358f))
        assertEquals(listOf(0f), fitColumns(listOf(0f), 358f))
    }
}
