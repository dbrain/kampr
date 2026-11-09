package dev.kampr.conversation.files

private const val INDENT = "  "

// Past this depth a line is indented no further: indenting every level of a deliberately deep
// document would make the output quadratic in its size.
private const val DEEPEST_INDENT = 32

private const val STRUCTURE = "{}[],:\"/"

// Laid out token by token rather than parsed, so a document cut off by a slice, or one that was
// never valid, comes out as far as it goes instead of not at all.
fun formatJson(text: String): String {
    val out = StringBuilder(text.length + text.length / 4)
    var depth = 0
    var owed = false
    var opened = false
    var gap = false
    var spaced = false
    var at = 0

    fun line() {
        out.append('\n')
        repeat(minOf(depth, DEEPEST_INDENT)) { out.append(INDENT) }
    }

    fun copyUntil(end: Int) {
        out.append(text, at, end)
        at = end
    }

    while (at < text.length) {
        val c = text[at]
        if (c.isWhitespace()) {
            gap = true
            at++
            continue
        }
        val closer = c == '}' || c == ']'
        when {
            closer -> {
                depth = maxOf(0, depth - 1)
                if (!opened && out.isNotEmpty()) line()
            }

            owed -> line()

            gap && spaced && (c !in STRUCTURE || c == '"' || text.startsWith("/*", at)) -> out.append(' ')
        }
        owed = false
        opened = false
        gap = false
        spaced = false
        when {
            closer -> out.append(text[at++])

            c == '{' || c == '[' -> {
                out.append(text[at++])
                depth++
                owed = true
                opened = true
            }

            c == ',' -> {
                out.append(text[at++])
                owed = true
            }

            c == ':' -> {
                out.append(": ")
                at++
            }

            c == '"' -> {
                var probe = at + 1
                while (probe < text.length && text[probe] != '"') probe += if (text[probe] == '\\') 2 else 1
                copyUntil(minOf(text.length, probe + 1))
                spaced = true
            }

            text.startsWith("//", at) -> {
                copyUntil(text.indexOf('\n', at).let { if (it < 0) text.length else it })
                owed = true
            }

            text.startsWith("/*", at) -> {
                copyUntil(text.indexOf("*/", at + 2).let { if (it < 0) text.length else it + 2 })
                spaced = true
            }

            else -> {
                var probe = at + 1
                while (probe < text.length && !text[probe].isWhitespace() && text[probe] !in STRUCTURE) probe++
                copyUntil(probe)
                spaced = true
            }
        }
    }
    return out.toString()
}
