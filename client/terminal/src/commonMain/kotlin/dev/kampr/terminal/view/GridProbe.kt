package dev.kampr.terminal.view

import androidx.compose.ui.geometry.Offset
import dev.kampr.shared.model.BLANK
import dev.kampr.shared.model.TAIL
import dev.kampr.terminal.render.GridPoint
import dev.kampr.terminal.render.SurfaceRows
import kotlin.math.floor

// Pointer positions have to become cells against the geometry of the frame the finger is on, and
// that geometry moves with every pan. A gesture detector keyed on it would restart mid-drag, so it
// reads this holder instead.
class GridProbe {
    var originX = 0f
    var originY = 0f
    var cellWidth = 1f
    var cellHeight = 1f
    var cols = 1
    var totalRows = 1

    fun cellAt(position: Offset): GridPoint {
        val col = floor((position.x - originX) / cellWidth).toInt().coerceIn(0, cols - 1)
        val row = floor((position.y - originY) / cellHeight).toInt().coerceIn(0, totalRows - 1)
        return GridPoint(row, col)
    }
}

// The span of the word under a cell, in columns: a run of printable glyphs bounded by
// whitespace. A tail is the second half of a wide glyph and reads as its lead, and a cell with
// nothing printable in it is its own span — a double-click on a gap selects the gap, which is
// what a text editor does and what makes the gesture predictable. The span is what a double-click
// (desk) and a long-press (touch) select, so the two surfaces answer the same press the same way.
fun wordAt(rows: SurfaceRows, point: GridPoint): Pair<Int, Int> {
    val row = point.row
    val cols = rows.cols
    val glyphs = IntArray(cols)
    val styles = IntArray(cols)
    if (!rows.into(row, glyphs, styles)) return point.col to point.col
    var col = point.col
    if (glyphs[col] == TAIL && col > 0) col -= 1
    fun isWord(c: Int): Boolean = c != BLANK && !c.toChar().isWhitespace()
    if (!isWord(glyphs[col])) return col to col
    var start = col
    while (start > 0 && isWord(glyphs[start - 1])) start--
    var end = col
    while (end < cols - 1 && isWord(glyphs[end + 1])) end++
    return start to end
}
