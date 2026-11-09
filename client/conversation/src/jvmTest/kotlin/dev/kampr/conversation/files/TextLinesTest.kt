package dev.kampr.conversation.files

import kotlin.test.Test
import kotlin.test.assertEquals

private fun bom(a: Int, b: Int) = byteArrayOf(a.toByte(), b.toByte())

class TextLinesTest {
    private val text = "añb\r\n€uro 𝄞 clef\n\nlast — no newline"

    private fun fed(sizes: Int, encoding: TextEncoding = TextEncoding.Utf8, bytes: ByteArray = text.encodeToByteArray()): TextLines {
        val lines = TextLines(encoding)
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
    fun aSliceIsReadInTheEncodingItsBytesShow() {
        val table = listOf(
            "utf-8" to "plain ✓".encodeToByteArray() to TextEncoding.Utf8,
            "utf-8 cut mid-character" to "cut €".encodeToByteArray().let { it.copyOf(it.size - 1) } to TextEncoding.Utf8,
            "utf-16le with its mark" to (bom(0xFF, 0xFE) + "Windows wrote this\r\n".toByteArray(Charsets.UTF_16LE)) to TextEncoding.Utf16Le,
            "utf-16be with its mark" to (bom(0xFE, 0xFF) + "big end".toByteArray(Charsets.UTF_16BE)) to TextEncoding.Utf16Be,
            "utf-16le with no mark" to "PowerShell > out.txt\r\n".toByteArray(Charsets.UTF_16LE) to TextEncoding.Utf16Le,
            "utf-16be with no mark" to "no mark here".toByteArray(Charsets.UTF_16BE) to TextEncoding.Utf16Be,
            "windows-1252" to "“café” — naïve".toByteArray(charset("windows-1252")) to TextEncoding.Windows1252,
            "binary with a nul" to byteArrayOf(0x61, 0, 0x62, 0x7F, 0x45, 0x4C, 0x46, 0, 0, 0, 1) to null,
            "elf header" to byteArrayOf(0x7F, 0x45, 0x4C, 0x46, 2, 1, 1, 0, 0, 0, 0, 0) to null,
            "control bytes, not utf-8" to byteArrayOf(0x61, 0xC3.toByte(), 0x28, 0x01, 0x02, 0x03, 0x62) to null,
        )
        for ((named, expected) in table) assertEquals(expected, textEncoding(named.second), named.first)
    }

    @Test
    fun linesAreTheSameWhereverTheSlicesCutAUtf16CharacterOrItsMark() {
        for ((encoding, charset, mark) in listOf(
            Triple(TextEncoding.Utf16Le, Charsets.UTF_16LE, bom(0xFF, 0xFE)),
            Triple(TextEncoding.Utf16Be, Charsets.UTF_16BE, bom(0xFE, 0xFF)),
        )) {
            val bytes = mark + text.toByteArray(charset)
            val whole = fed(bytes.size, encoding, bytes)
            assertEquals(listOf("añb\r", "€uro 𝄞 clef", "", "last — no newline"), whole.lines, "$encoding whole")
            for (size in 1..9) assertEquals(whole.lines, fed(size, encoding, bytes).lines, "$encoding in slices of $size bytes")
            assertEquals(bytes.size.toLong(), whole.bytes, "$encoding counts the bytes it was given, not the characters")
        }
    }

    @Test
    fun windows1252IsReadByteForByteIncludingItsPunctuation() {
        val words = "“quoted” – €5 … naïve\nsecond"
        val lines = fed(3, TextEncoding.Windows1252, words.toByteArray(charset("windows-1252")))
        assertEquals(listOf("“quoted” – €5 … naïve", "second"), lines.lines)
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
