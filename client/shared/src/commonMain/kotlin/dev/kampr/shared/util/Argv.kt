package dev.kampr.shared.util

// `agent.start` carries argv, not a command line, and herdr execs it — no shell is involved. So
// the only shell grammar honoured here is the one somebody typing flags actually uses: spaces
// separate, quotes group. Anything else would promise an expansion that never happens.
fun parseArgs(typed: String): List<String> {
    val args = mutableListOf<String>()
    val current = StringBuilder()
    var quote: Char? = null
    var open = false
    for (ch in typed) {
        when {
            quote != null && ch == quote -> {
                quote = null
            }

            quote != null -> current.append(ch)

            ch == '"' || ch == '\'' -> {
                quote = ch
                open = true
            }

            ch.isWhitespace() -> {
                if (current.isNotEmpty() || open) args += current.toString()
                current.clear()
                open = false
            }

            else -> current.append(ch)
        }
    }
    if (current.isNotEmpty() || open) args += current.toString()
    return args
}

// docs/25 and Rex's resume spec: a stored command is shown exactly as it would run, in the quoting a
// shell would read back to the same argv, and with nothing left in it that a terminal or a text
// layout would act on — a control character, a bidi override or a zero-width mark.
fun displayArgv(argv: List<String>): String = argv.joinToString(" ", transform = ::displayArg)

private const val PLAIN = "_@%+=:,./-"

private fun displayArg(arg: String): String = when {
    arg.isNotEmpty() && arg.all { it.isAsciiLetterOrDigit() || it in PLAIN } -> arg
    arg.any(::unseen) -> "$'" + arg.map(::escaped).joinToString("") + "'"
    else -> "'" + arg.replace("'", "'\\''") + "'"
}

private fun Char.isAsciiLetterOrDigit(): Boolean = this in 'a'..'z' || this in 'A'..'Z' || this in '0'..'9'

private fun unseen(c: Char): Boolean = c.category in UNSEEN

private val UNSEEN = setOf(
    CharCategory.CONTROL,
    CharCategory.FORMAT,
    CharCategory.LINE_SEPARATOR,
    CharCategory.PARAGRAPH_SEPARATOR,
    CharCategory.UNASSIGNED,
    CharCategory.PRIVATE_USE,
)

private fun escaped(c: Char): String = when {
    c == '\n' -> "\\n"
    c == '\t' -> "\\t"
    c == '\r' -> "\\r"
    c == '\\' -> "\\\\"
    c == '\'' -> "\\'"
    !unseen(c) -> c.toString()
    c.code < 0x80 -> "\\x" + c.code.toString(16).padStart(2, '0')
    else -> "\\u{" + c.code.toString(16) + "}"
}

fun commandLine(kind: String, args: List<String>): String = (listOf(kind) + args.map { if (it.any(Char::isWhitespace)) "\"$it\"" else it }).joinToString(" ")

// Named, not guessed at: every one of these is a flag some harness ships that removes the
// confirmation an operator would otherwise get. A remembered flag is invisible by the second
// launch unless something says what it does, and this is what says it.
private val BYPASS = listOf(
    "dangerously",
    "yolo",
    "full-auto",
    "auto-approve",
    "autoapprove",
    "no-sandbox",
    "skip-permissions",
    "bypass",
)

fun bypassesSafety(arg: String): Boolean {
    val flag = arg.substringBefore('=').lowercase()
    return BYPASS.any { it in flag }
}
