package dev.kampr.shared.model

import dev.kampr.shared.wire.RowDiff

class CellBuffer(cols: Int, rows: Int) {
    var cols: Int = cols
        private set
    var rows: Int = rows
        private set

    var chars: CharArray = CharArray(cols * rows) { ' ' }
        private set
    var styles: ShortArray = ShortArray(cols * rows)
        private set
    var links: IntArray = IntArray(cols * rows)
        private set

    private var dirty = BooleanArray(rows) { true }

    fun resize(newCols: Int, newRows: Int) {
        if (newCols == cols && newRows == rows) return
        cols = newCols
        rows = newRows
        chars = CharArray(cols * rows) { ' ' }
        styles = ShortArray(cols * rows)
        links = IntArray(cols * rows)
        dirty = BooleanArray(rows) { true }
    }

    fun clear() {
        chars.fill(' ')
        styles.fill(0)
        links.fill(0)
        dirty.fill(true)
    }

    fun isDirty(row: Int): Boolean = row in 0 until rows && dirty[row]

    fun clearDirty() = dirty.fill(false)

    fun apply(diff: RowDiff) {
        val row = diff.row
        if (row < 0 || row >= rows) return
        val base = row * cols
        var col = 0
        for (run in diff.runs) {
            val style = run.s.toShort()
            val link = run.l?.plus(1) ?: 0
            for (ch in run.x) {
                if (col >= cols) break
                chars[base + col] = ch
                styles[base + col] = style
                links[base + col] = link
                col++
            }
            if (col >= cols) break
        }
        while (col < cols) {
            chars[base + col] = ' '
            styles[base + col] = 0
            links[base + col] = 0
            col++
        }
        dirty[row] = true
    }

    fun charAt(col: Int, row: Int): Char = chars[row * cols + col]

    fun styleAt(col: Int, row: Int): Int = styles[row * cols + col].toInt()

    fun linkAt(col: Int, row: Int): Int = links[row * cols + col] - 1
}
