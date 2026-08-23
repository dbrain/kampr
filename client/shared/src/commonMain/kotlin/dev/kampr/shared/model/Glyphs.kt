package dev.kampr.shared.model

// Probe #210: a cell holds a code point, not a UTF-16 unit — an astral glyph split across two
// cells is two surrogate halves, each of which renders as nothing. TAIL is the right half of a
// double-width glyph: a column that belongs to its neighbour and draws nothing of its own.
const val TAIL = 0
const val BLANK = 32

fun glyphAt(text: String, index: Int): Int {
    val high = text[index]
    if (!high.isHighSurrogate() || index + 1 >= text.length) return high.code
    val low = text[index + 1]
    if (!low.isLowSurrogate()) return high.code
    return 0x10000 + ((high.code - 0xD800) shl 10) + (low.code - 0xDC00)
}

fun glyphUnits(codePoint: Int): Int = if (codePoint >= 0x10000) 2 else 1

fun StringBuilder.appendGlyph(codePoint: Int): StringBuilder {
    if (codePoint < 0x10000) return append(codePoint.toChar())
    val v = codePoint - 0x10000
    append((0xD800 + (v shr 10)).toChar())
    return append((0xDC00 + (v and 0x3FF)).toChar())
}

fun glyphString(codePoint: Int): String = StringBuilder(2).appendGlyph(codePoint).toString()
