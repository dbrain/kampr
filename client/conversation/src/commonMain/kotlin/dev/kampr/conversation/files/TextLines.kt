package dev.kampr.conversation.files

// Where the bytes stop being whole characters: a slice cut at a byte count, not a character
// boundary, can end one to three bytes into a UTF-8 sequence, and those bytes belong to the next.
internal fun completeUpTo(bytes: ByteArray, size: Int = bytes.size): Int {
    for (i in size - 1 downTo maxOf(0, size - 4)) {
        val b = bytes[i].toInt() and 0xFF
        if (b and 0xC0 == 0x80) continue
        val needs = when {
            b < 0x80 -> 1
            b and 0xE0 == 0xC0 -> 2
            b and 0xF0 == 0xE0 -> 3
            b and 0xF8 == 0xF0 -> 4
            else -> 1
        }
        return if (i + needs > size) i else size
    }
    return size
}

// A file arriving in slices, kept as the lines it has so far. A line is only a line once its
// newline has arrived: the text after the last one is held back until the next slice or the end.
// A `\r` before the newline is kept, so the text copied is the text that was there.
class TextLines(val encoding: TextEncoding = TextEncoding.Utf8) {
    private val done = ArrayList<String>()
    private var tail = ""
    private var carry = ByteArray(0)
    private var trailing = false
    private var started = false

    var bytes = 0L
        private set
    var widest = 0
        private set
    var ended = false
        private set

    val lines: List<String> get() = done

    fun append(slice: ByteArray) {
        bytes += slice.size
        val joined = if (carry.isEmpty()) slice else carry + slice
        val cut = encoding.wholeUpTo(joined)
        carry = joined.copyOfRange(cut, joined.size)
        val parts = (tail + decoded(joined, cut)).split('\n')
        for (i in 0 until parts.lastIndex) add(parts[i])
        tail = parts.last()
    }

    fun end() {
        if (ended) return
        ended = true
        if (carry.isNotEmpty()) tail += decoded(carry, carry.size)
        carry = ByteArray(0)
        trailing = tail.isEmpty() && done.isNotEmpty()
        if (tail.isNotEmpty()) add(tail)
        tail = ""
    }

    fun text(): String = done.joinToString("\n", postfix = if (trailing) "\n" else "")

    private fun decoded(bytes: ByteArray, to: Int): String = encoding.decode(bytes, to, first = !started).also {
        if (it.isNotEmpty()) started = true
    }

    private fun add(line: String) {
        done += line
        if (line.length > widest) widest = line.length
    }
}
