package dev.kampr.terminal.render

import dev.kampr.shared.model.BLANK
import dev.kampr.shared.model.TAIL
import dev.kampr.shared.model.appendGlyph
import dev.kampr.shared.model.glyphUnits
import dev.kampr.shared.net.filePathOf

data class GridPoint(val row: Int, val col: Int) : Comparable<GridPoint> {
    override fun compareTo(other: GridPoint): Int =
        if (row != other.row) row - other.row else col - other.col
}

// Linear by default: the selection flows from the first cell to the last across the rows between,
// the way selecting a paragraph does. Block is the secondary mode, for columnar output.
data class Selection(val anchor: GridPoint, val head: GridPoint, val block: Boolean = false) {
    val start: GridPoint get() = if (anchor <= head) anchor else head
    val end: GridPoint get() = if (anchor <= head) head else anchor
    val empty: Boolean get() = anchor == head

    fun span(row: Int, cols: Int): IntRange? {
        val first = start
        val last = end
        if (row < first.row || row > last.row) return null
        if (block) {
            val lo = minOf(first.col, last.col)
            val hi = maxOf(first.col, last.col)
            return lo..hi.coerceAtMost(cols - 1)
        }
        val from = if (row == first.row) first.col else 0
        val to = if (row == last.row) last.col else cols - 1
        if (from > to) return null
        return from..to.coerceAtMost(cols - 1)
    }
}

// A row whose last cell is not blank continued into the next one, so joining them without a
// newline is what reconstructs the logical line. A path or a URL broken by a newline in the middle
// is worse than not copying it at all.
class LogicalText(private val rows: SurfaceRows) {
    private var glyphs = IntArray(0)
    private var styles = IntArray(0)
    private var links = IntArray(0)
    private var marks = emptyArray<String>()

    // The pane is 80x24 until its first grid.reset, so the scratch has to follow the real width
    // rather than the one that happened to be there when the surface was created.
    private fun read(index: Int): Boolean {
        val cols = rows.cols
        if (cols == 0) return false
        if (glyphs.size < cols) {
            glyphs = IntArray(cols)
            styles = IntArray(cols)
            links = IntArray(cols)
            marks = Array(cols) { "" }
        }
        return rows.into(index, glyphs, styles, links, marks)
    }

    private fun wraps(index: Int): Boolean {
        if (!read(index)) return false
        return glyphs[rows.cols - 1] != BLANK
    }

    private fun lastInk(): Int {
        var end = rows.cols - 1
        while (end >= 0 && glyphs[end] == BLANK) end--
        return end
    }

    private fun appendInk(builder: StringBuilder, from: Int, to: Int) {
        for (i in from..to) if (glyphs[i] != TAIL) builder.appendGlyph(glyphs[i]).append(marks[i])
    }

    // What column `col` costs the string appendInk builds: its base in UTF-16 units, plus whatever
    // it is wearing. A column is not a string offset once either can be more than one unit.
    private fun units(col: Int): Int =
        if (glyphs[col] == TAIL) 0 else glyphUnits(glyphs[col]) + marks[col].length

    // Probe #210: a double-width glyph owns two columns, so the column a finger lands on is not
    // always the column its glyph starts in. Anything turning a column into a character resolves
    // that first, or it is a glyph out. Reads the row already in the scratch.
    private fun lead(col: Int): Int = if (col > 0 && glyphs[col] == TAIL) col - 1 else col

    fun copy(selection: Selection): String {
        val cols = rows.cols
        if (cols == 0) return ""
        val builder = StringBuilder()
        val first = selection.start.row
        val last = selection.end.row
        for (row in first..last) {
            val span = selection.span(row, cols) ?: continue
            if (!read(row)) continue
            val from = lead(span.first)
            var end = span.last
            while (end >= from && glyphs[end] == BLANK) end--
            if (end >= from) appendInk(builder, from, end)
            if (row == last) break
            if (selection.block || !wraps(row)) builder.append('\n')
        }
        return builder.toString()
    }

    // One row's own cells, trailing blanks trimmed. Review walks the grid a row at a time
    // rather than a logical line at a time: a full-screen TUI's rows are what is on the screen,
    // and joining them across a wrap would interleave two columns of a split layout.
    fun rowAt(index: Int): String {
        val cols = rows.cols
        if (cols == 0 || !read(index)) return ""
        val end = lastInk()
        if (end < 0) return ""
        val builder = StringBuilder(end + 1)
        appendInk(builder, 0, end)
        return builder.toString()
    }

    // The logical line through a row, and the offset *in that string* of the column asked for.
    // Column and offset are two different coordinates once a glyph can be two columns wide, and
    // handing a caller the column would put a link detector a character out on every CJK line.
    fun lineAt(index: Int, col: Int): Pair<String, Int> {
        val cols = rows.cols
        if (cols == 0) return "" to 0
        var first = index
        while (first > 0 && wraps(first - 1)) first--
        val builder = StringBuilder()
        var offset = 0
        var row = first
        while (true) {
            if (!read(row)) break
            val end = lastInk()
            if (row == index) {
                val target = lead(col.coerceIn(0, cols - 1))
                var at = builder.length
                for (i in 0 until minOf(target, end + 1)) at += units(i)
                offset = at
            }
            if (end >= 0) appendInk(builder, 0, end)
            if (!wraps(row)) break
            row++
        }
        return builder.toString() to offset
    }

    fun linkAt(index: Int, col: Int): Int {
        if (!read(index) || col !in 0 until rows.cols) return -1
        return links[col]
    }
}

// Pane output is attacker-influenceable, so detection is a strict scheme match rather than
// "anything with a dot in it", and a hit is only ever offered, never navigated to.
private val URL = Regex("""\b(?:https?|ftp|file)://[^\s"'`<>()\[\]{}]+""")
private val PATH = Regex("""\b[\w.\-/]+\.[A-Za-z][\w]{0,7}:\d+(?::\d+)?\b""")

// Where a shell, a compiler or an agent stops writing a path: whitespace, and the brackets and
// quotes every one of them wraps one in.
private const val BREAKS = "\"'`<>()[]{},;|"

// `main.rs:412:9` is a path and a place in it. The place is the compiler's; the node opens the path.
private val LOCATION = Regex(""":\d+(?::\d+)?$""")

// `File` is a path the route can fetch — absolute or `~/`-rooted, which is all `filePathOf`
// accepts. `Path` is the weaker thing beside it: a `file.rs:12` reference a reader can copy but
// nothing can resolve, because a relative path has no directory to be relative to from here.
enum class TargetKind { Link, Url, Path, File }

data class Target(val text: String, val kind: TargetKind)

private fun breaks(c: Char): Boolean = c.isWhitespace() || c in BREAKS

// The token under the finger, with the compiler's location and the sentence's punctuation taken
// off it, and only if `filePathOf` — the same arbiter the conversation surface uses — will have it.
// Deliberately not a search through prose: a bare `foo.rs` is a guess about English, and a guess
// that offers to fetch a file is worse than not offering one.
fun detectPath(line: String, offset: Int): String? {
    if (offset !in line.indices || breaks(line[offset])) return null
    var start = offset
    while (start > 0 && !breaks(line[start - 1])) start--
    var end = offset + 1
    while (end < line.length && !breaks(line[end])) end++
    val token = line.substring(start, end).trimEnd('.', ',', ';', ':', '!', '?')
    return filePathOf(LOCATION.replace(token, ""))
}

fun detectTarget(line: String, offset: Int): Target? {
    for (match in URL.findAll(line)) {
        if (offset in match.range) return Target(match.value.trimEnd('.', ',', ';', ':', '!', '?'), TargetKind.Url)
    }
    detectPath(line, offset)?.let { return Target(it, TargetKind.File) }
    for (match in PATH.findAll(line)) {
        if (offset in match.range) return Target(match.value, TargetKind.Path)
    }
    return null
}
