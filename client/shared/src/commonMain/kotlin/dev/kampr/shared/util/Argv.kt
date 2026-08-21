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

fun commandLine(kind: String, args: List<String>): String =
    (listOf(kind) + args.map { if (it.any(Char::isWhitespace)) "\"$it\"" else it }).joinToString(" ")

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
