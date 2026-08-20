package dev.kampr.terminal.render

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
    private var chars = CharArray(0)
    private var styles = IntArray(0)
    private var links = IntArray(0)

    // The pane is 80x24 until its first grid.reset, so the scratch has to follow the real width
    // rather than the one that happened to be there when the surface was created.
    private fun read(index: Int): Boolean {
        val cols = rows.cols
        if (cols == 0) return false
        if (chars.size < cols) {
            chars = CharArray(cols)
            styles = IntArray(cols)
            links = IntArray(cols)
        }
        return rows.into(index, chars, styles, links)
    }

    private fun wraps(index: Int): Boolean {
        if (!read(index)) return false
        return chars[rows.cols - 1] != ' '
    }

    fun copy(selection: Selection): String {
        val cols = rows.cols
        if (cols == 0) return ""
        val builder = StringBuilder()
        val first = selection.start.row
        val last = selection.end.row
        for (row in first..last) {
            val span = selection.span(row, cols) ?: continue
            if (!read(row)) continue
            var end = span.last
            while (end >= span.first && chars[end] == ' ') end--
            for (i in span.first..end) builder.append(chars[i])
            if (row == last) break
            if (selection.block || !wraps(row)) builder.append('\n')
        }
        return builder.toString()
    }

    // The logical line through a row, plus the offset at which that row's own cells begin.
    fun lineAt(index: Int): Pair<String, Int> {
        val cols = rows.cols
        if (cols == 0) return "" to 0
        var first = index
        while (first > 0 && wraps(first - 1)) first--
        val builder = StringBuilder()
        var offset = 0
        var row = first
        while (true) {
            if (row == index) offset = builder.length
            if (!read(row)) break
            var end = cols - 1
            while (end >= 0 && chars[end] == ' ') end--
            for (i in 0..end) builder.append(chars[i])
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

enum class TargetKind { Link, Url, Path }

data class Target(val text: String, val kind: TargetKind)

fun detectTarget(line: String, offset: Int): Target? {
    for (match in URL.findAll(line)) {
        if (offset in match.range) return Target(match.value.trimEnd('.', ',', ';', ':', '!', '?'), TargetKind.Url)
    }
    for (match in PATH.findAll(line)) {
        if (offset in match.range) return Target(match.value, TargetKind.Path)
    }
    return null
}
