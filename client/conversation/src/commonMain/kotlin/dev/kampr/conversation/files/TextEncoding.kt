package dev.kampr.conversation.files

enum class TextEncoding(val label: String?) {
    Utf8(null),
    Utf16Le("UTF-16 LE"),
    Utf16Be("UTF-16 BE"),
    Windows1252("Windows-1252"),
}

// How far into a file its encoding is judged from: enough pairs to see a pattern of NULs, and short
// enough that a 2 MiB first slice is not walked twice.
private const val JUDGED = 4096

// Windows-1252's 0x80–0x9F, the only part of it that is not Latin-1. Zero is a byte the code page
// leaves undefined, and a file holding one is not text in it.
private val CP1252_HIGH = intArrayOf(
    0x20AC, 0, 0x201A, 0x0192, 0x201E, 0x2026, 0x2020, 0x2021, 0x02C6, 0x2030, 0x0160, 0x2039, 0x0152, 0, 0x017D, 0,
    0, 0x2018, 0x2019, 0x201C, 0x201D, 0x2022, 0x2013, 0x2014, 0x02DC, 0x2122, 0x0161, 0x203A, 0x0153, 0, 0x017E, 0x0178,
)

// A C0 control other than the whitespace a text file holds and the escape that colours a log is
// the first sign of a binary file, and it is everywhere in one.
private fun isControl(c: Int) = c < 0x20 && c != 0x09 && c != 0x0A && c != 0x0D && c != 0x0C && c != 0x1B

private fun at(bytes: ByteArray, i: Int) = bytes[i].toInt() and 0xFF

// UTF-8 first, because it is what nearly everything is and a valid UTF-8 slice is almost never
// anything else. A NUL rules UTF-8 and Windows-1252 out, and leaves UTF-16 — what PowerShell's `>`,
// Notepad's "Unicode" and many Windows tools write — which ASCII-heavy text shows as a NUL in every
// other byte. What is neither is read as Windows-1252, the encoding an old Windows file is in when
// it is not UTF-8, unless a control byte says it is not text at all.
fun textEncoding(bytes: ByteArray): TextEncoding? {
    if (bytes.size >= 2 && at(bytes, 0) == 0xFF && at(bytes, 1) == 0xFE) return TextEncoding.Utf16Le
    if (bytes.size >= 2 && at(bytes, 0) == 0xFE && at(bytes, 1) == 0xFF) return TextEncoding.Utf16Be
    val judged = minOf(bytes.size, JUDGED)
    if ((0 until judged).any { bytes[it] == 0.toByte() }) return utf16(bytes, judged)
    val cut = completeUpTo(bytes)
    if (runCatching { bytes.decodeToString(0, cut, throwOnInvalidSequence = true) }.isSuccess) return TextEncoding.Utf8
    val cp1252 = (0 until judged).none { i ->
        val b = at(bytes, i)
        isControl(b) || (b in 0x80..0x9F && CP1252_HIGH[b - 0x80] == 0)
    }
    return if (cp1252) TextEncoding.Windows1252 else null
}

private fun utf16(bytes: ByteArray, judged: Int): TextEncoding? {
    val pairs = judged / 2
    if (pairs == 0) return null
    val evenNul = (0 until pairs).count { bytes[2 * it] == 0.toByte() }
    val oddNul = (0 until pairs).count { bytes[2 * it + 1] == 0.toByte() }
    val encoding = when {
        oddNul * 10 >= pairs * 4 && evenNul * 20 <= pairs -> TextEncoding.Utf16Le
        evenNul * 10 >= pairs * 4 && oddNul * 20 <= pairs -> TextEncoding.Utf16Be
        else -> return null
    }
    val units = decodeUtf16(bytes, 0, pairs * 2, encoding == TextEncoding.Utf16Le)
    return encoding.takeIf { units.none { isControl(it.code) } }
}

private fun decodeUtf16(bytes: ByteArray, from: Int, to: Int, little: Boolean): String {
    val chars = CharArray((to - from) / 2) { i ->
        val lo = at(bytes, from + 2 * i)
        val hi = at(bytes, from + 2 * i + 1)
        (if (little) (hi shl 8) or lo else (lo shl 8) or hi).toChar()
    }
    return chars.concatToString()
}

// Where the bytes stop being whole characters in this encoding. A slice is cut at a byte count, so
// it can end inside a UTF-8 sequence or between the two bytes of a UTF-16 unit; whatever is past
// the cut belongs to the next slice. A surrogate pair cut in two needs nothing: a Kotlin string is
// UTF-16 units already, and the two halves join when the lines are.
internal fun TextEncoding.wholeUpTo(bytes: ByteArray): Int = when (this) {
    TextEncoding.Utf8 -> completeUpTo(bytes)
    TextEncoding.Windows1252 -> bytes.size
    TextEncoding.Utf16Le, TextEncoding.Utf16Be -> bytes.size - bytes.size % 2
}

// `first` is the start of the file, where a UTF-16 byte-order mark is the encoding's and not text.
internal fun TextEncoding.decode(bytes: ByteArray, to: Int, first: Boolean): String = when (this) {
    TextEncoding.Utf8 -> bytes.decodeToString(0, to)

    TextEncoding.Windows1252 -> CharArray(to) { i ->
        val b = at(bytes, i)
        if (b in 0x80..0x9F) (CP1252_HIGH[b - 0x80].takeIf { it != 0 } ?: 0xFFFD).toChar() else b.toChar()
    }.concatToString()

    TextEncoding.Utf16Le, TextEncoding.Utf16Be -> {
        val text = decodeUtf16(bytes, 0, to - to % 2, this == TextEncoding.Utf16Le) + if (to % 2 == 1) "\uFFFD" else ""
        if (first && text.startsWith('\uFEFF')) text.substring(1) else text
    }
}
