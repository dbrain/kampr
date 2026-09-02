package dev.kampr.conversation.md

private val FENCE = Regex("^ {0,3}(`{3,}|~{3,})\\s*([^`\\s]*).*$")
private val ATX = Regex("^ {0,3}(#{1,6})\\s+(.*?)\\s*#*\\s*$")
private val RULE = Regex("^ {0,3}([-*_])(\\s*\\1){2,}\\s*$")
private val BULLET = Regex("^(\\s*)([-*+])\\s+(.*)$")
private val ORDERED = Regex("^(\\s*)(\\d{1,9})[.)]\\s+(.*)$")
private val DELIMITER = Regex("^\\s*\\|?\\s*:?-{1,}:?\\s*(\\|\\s*:?-{1,}:?\\s*)*\\|?\\s*$")

// What a single newline inside a paragraph means. CommonMark says a space, and that is right for
// an agent: it writes real markdown and wraps its own prose, so a line that ended because the
// column did must not become a break. It is wrong for a prompt a person typed, where Enter was
// pressed on purpose — which the wire says outright with `role: user`
// (docs/04-wire-protocol.md). Nothing else about the parse changes: a person pastes fences,
// bullets and tables into a reply and they go on rendering as what they are.
enum class Breaks { Soft, Hard }

fun parseMarkdown(source: String, breaks: Breaks = Breaks.Soft): List<MdBlock> =
    Reader(source.replace("\r\n", "\n").split('\n'), breaks).blocks()

private class Reader(private val lines: List<String>, private val breaks: Breaks) {
    private var at = 0

    fun blocks(): List<MdBlock> {
        val out = mutableListOf<MdBlock>()
        while (at < lines.size) {
            val line = lines[at]
            when {
                line.isBlank() -> at++
                FENCE.matches(line) -> out += fence()
                RULE.matches(line) -> { at++; out += MdBlock.Rule }
                ATX.matches(line) -> out += heading()
                line.trimStart().startsWith("> ") || line.trimStart() == ">" -> out += quote()
                isTable() -> out += table()
                BULLET.matches(line) || ORDERED.matches(line) -> out += list()
                else -> out += paragraph()
            }
        }
        return out
    }

    private fun fence(): MdBlock {
        val open = FENCE.matchEntire(lines[at])!!
        val marker = open.groupValues[1]
        val lang = open.groupValues[2].takeIf { it.isNotBlank() }
        at++
        val body = mutableListOf<String>()
        while (at < lines.size && !lines[at].trimStart().startsWith(marker)) {
            body += lines[at]
            at++
        }
        if (at < lines.size) at++
        return MdBlock.Fence(lang, body.joinToString("\n"))
    }

    private fun heading(): MdBlock {
        val match = ATX.matchEntire(lines[at])!!
        at++
        return MdBlock.Heading(match.groupValues[1].length, match.groupValues[2])
    }

    private fun quote(): MdBlock {
        val body = mutableListOf<String>()
        while (at < lines.size) {
            val trimmed = lines[at].trimStart()
            if (!trimmed.startsWith(">")) break
            body += trimmed.removePrefix(">").removePrefix(" ")
            at++
        }
        return MdBlock.Quote(Reader(body, breaks).blocks())
    }

    private fun isTable(): Boolean {
        val head = lines.getOrNull(at) ?: return false
        val delimiter = lines.getOrNull(at + 1) ?: return false
        if (!head.contains('|')) return false
        if (!delimiter.contains('-') || !DELIMITER.matches(delimiter)) return false
        return cells(head).size == cells(delimiter).size
    }

    private fun table(): MdBlock {
        val header = cells(lines[at])
        val aligns = cells(lines[at + 1]).map {
            val left = it.startsWith(":")
            val right = it.endsWith(":")
            when {
                left && right -> Align.Center
                right -> Align.End
                else -> Align.Start
            }
        }
        at += 2
        val rows = mutableListOf<List<String>>()
        while (at < lines.size && lines[at].contains('|') && lines[at].isNotBlank()) {
            val row = cells(lines[at])
            rows += List(header.size) { row.getOrElse(it) { "" } }
            at++
        }
        return MdBlock.Table(header, rows, aligns)
    }

    // Splitting on a bare `|` would cut a cell containing an escaped pipe or a pipe inside a
    // code span, both of which are ordinary in agent output describing shell pipelines.
    private fun cells(line: String): List<String> {
        val out = mutableListOf<String>()
        val cell = StringBuilder()
        var inCode = false
        var index = 0
        val body = line.trim().removePrefix("|").removeSuffix("|")
        while (index < body.length) {
            val c = body[index]
            when {
                c == '\\' && index + 1 < body.length -> {
                    cell.append(body[index + 1]); index++
                }
                c == '`' -> { inCode = !inCode; cell.append(c) }
                c == '|' && !inCode -> { out += cell.toString().trim(); cell.setLength(0) }
                else -> cell.append(c)
            }
            index++
        }
        out += cell.toString().trim()
        return out
    }

    private fun list(): MdBlock {
        val first = BULLET.matchEntire(lines[at]) ?: ORDERED.matchEntire(lines[at])!!
        val ordered = ORDERED.matches(lines[at])
        val indent = first.groupValues[1].length
        val start = if (ordered) first.groupValues[2].toIntOrNull() ?: 1 else 1
        val items = mutableListOf<MdItem>()
        var counter = start
        while (at < lines.size) {
            val line = lines[at]
            val match = BULLET.matchEntire(line) ?: ORDERED.matchEntire(line) ?: break
            if (match.groupValues[1].length < indent) break
            if (match.groupValues[1].length > indent) break
            val isOrdered = ORDERED.matches(line)
            if (isOrdered != ordered) break
            val body = mutableListOf(match.groupValues[3])
            at++
            while (at < lines.size) {
                val next = lines[at]
                if (next.isBlank()) {
                    val following = lines.getOrNull(at + 1)
                    if (following == null || following.isBlank() || leading(following) <= indent) break
                    body += ""
                    at++
                    continue
                }
                if (leading(next) <= indent && (BULLET.matches(next) || ORDERED.matches(next))) break
                if (leading(next) <= indent && next.isNotBlank() && body.lastOrNull()?.isBlank() == true) break
                body += next.removePrefix(" ".repeat(minOf(leading(next), indent + 2)))
                at++
            }
            items += MdItem(if (ordered) "${counter}." else "•", Reader(body, breaks).blocks())
            counter++
        }
        return MdBlock.Bullets(items, ordered)
    }

    private fun leading(line: String): Int = line.length - line.trimStart().length

    // Two trailing spaces are CommonMark's own hard break, and they never worked here: the line was
    // trimmed before anything could read them and the join put a space back. They are honoured in
    // both modes now, which is what a writer who typed them asked for either way.
    private fun paragraph(): MdBlock {
        val body = StringBuilder()
        var broke = false
        while (at < lines.size) {
            val line = lines[at]
            if (line.isBlank() || FENCE.matches(line) || ATX.matches(line) || RULE.matches(line)) break
            if (line.trimStart().startsWith("> ")) break
            if (BULLET.matches(line) || ORDERED.matches(line)) break
            if (isTable()) break
            if (body.isNotEmpty()) body.append(if (broke) '\n' else ' ')
            body.append(line.trim())
            broke = breaks == Breaks.Hard || line.endsWith("  ")
            at++
        }
        return MdBlock.Paragraph(body.toString())
    }
}
