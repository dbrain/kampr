package dev.kampr.conversation.files

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

class TextLinesTest {
    private val text = "añb\r\n€uro 𝄞 clef\n\nlast — no newline"

    private fun fed(sizes: Int): TextLines {
        val bytes = text.encodeToByteArray()
        val lines = TextLines()
        bytes.toList().chunked(sizes).forEach { lines.append(it.toByteArray()) }
        lines.end()
        return lines
    }

    @Test
    fun linesAreTheSameWhereverTheSlicesCutACharacter() {
        val whole = fed(text.encodeToByteArray().size)
        assertEquals(listOf("añb\r", "€uro 𝄞 clef", "", "last — no newline"), whole.lines)
        for (size in 1..9) assertEquals(whole.lines, fed(size).lines, "slices of $size bytes")
    }

    @Test
    fun theLastLineIsHeldBackUntilTheEndAndTheNewlineAtTheEndIsNotALine() {
        val lines = TextLines()
        lines.append("one\ntw".encodeToByteArray())
        assertEquals(listOf("one"), lines.lines)
        lines.append("o\n".encodeToByteArray())
        assertEquals(listOf("one", "two"), lines.lines)
        lines.end()
        assertEquals(listOf("one", "two"), lines.lines)
        assertEquals("one\ntwo\n", lines.text())
        assertEquals(8L, lines.bytes)
    }

    @Test
    fun aSliceIsWordsUnlessItHasANulOrBrokenUtf8BeforeItsTail() {
        assertTrue(looksLikeText("plain ✓".encodeToByteArray()))
        assertTrue(looksLikeText("cut €".encodeToByteArray().let { it.copyOf(it.size - 1) }))
        assertFalse(looksLikeText(byteArrayOf(0x61, 0, 0x62)))
        assertFalse(looksLikeText(byteArrayOf(0x61, 0xC3.toByte(), 0x28, 0x62)))
    }

    @Test
    fun theWidestLineIsTrackedAsLinesArrive() {
        val lines = TextLines()
        lines.append("ab\nabcdef\nabc".encodeToByteArray())
        assertEquals(6, lines.widest)
        lines.end()
        assertEquals(6, lines.widest)
    }
}
