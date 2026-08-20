package dev.kampr.terminal.input

// Probes #8/#9: herdr's key grammar rejects Home, End, PageUp, PageDown, Insert, Delete and
// BackTab on 0.8.2, but pane.send_text writes raw bytes, so every one of them is reachable as
// its escape sequence. Nothing in the key row goes through send_keys.
object Esc {
    const val ESCAPE = "\u001b"
    const val TAB = "\t"
    const val BACKTAB = "\u001b[Z"
    const val ENTER = "\r"
    const val BACKSPACE = "\u007f"

    const val UP = "\u001b[A"
    const val DOWN = "\u001b[B"
    const val RIGHT = "\u001b[C"
    const val LEFT = "\u001b[D"

    const val HOME = "\u001b[H"
    const val END = "\u001b[F"
    const val PAGE_UP = "\u001b[5~"
    const val PAGE_DOWN = "\u001b[6~"
    const val INSERT = "\u001b[2~"
    const val DELETE = "\u001b[3~"

    private val functions = arrayOf(
        "\u001bOP", "\u001bOQ", "\u001bOR", "\u001bOS",
        "\u001b[15~", "\u001b[17~", "\u001b[18~", "\u001b[19~",
        "\u001b[20~", "\u001b[21~", "\u001b[23~", "\u001b[24~",
    )

    fun function(n: Int): String = functions[(n - 1).coerceIn(0, functions.size - 1)]

    fun control(ch: Char): String? {
        val lower = ch.lowercaseChar()
        return when {
            lower in 'a'..'z' -> (lower - 'a' + 1).toChar().toString()
            ch == ' ' || ch == '@' -> "\u0000"
            ch == '[' -> ESCAPE
            ch == '\\' -> "\u001c"
            ch == ']' -> "\u001d"
            ch == '^' -> "\u001e"
            ch == '_' || ch == '-' -> "\u001f"
            ch == '?' -> BACKSPACE
            else -> null
        }
    }

    // CSI sequences carry their modifier as a parameter rather than an ESC prefix: 2 shift,
    // 3 alt, 5 ctrl, summed the way xterm does it.
    fun modified(sequence: String, ctrl: Boolean, alt: Boolean, shift: Boolean): String {
        if (!ctrl && !alt && !shift) return sequence
        var code = 1
        if (shift) code += 1
        if (alt) code += 2
        if (ctrl) code += 4
        val final = sequence.lastOrNull() ?: return sequence
        return when {
            sequence.length == 3 && (sequence[1] == '[' || sequence[1] == 'O') -> "\u001b[1;$code$final"
            sequence.endsWith("~") -> sequence.dropLast(1) + ";$code~"
            else -> sequence
        }
    }
}
