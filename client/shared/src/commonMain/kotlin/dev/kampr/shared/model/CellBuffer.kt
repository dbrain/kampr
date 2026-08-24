package dev.kampr.shared.model

import dev.kampr.shared.wire.RowDiff

class CellBuffer(cols: Int, rows: Int) {
    var cols: Int = cols
        private set
    var rows: Int = rows
        private set

    var glyphs: IntArray = IntArray(cols * rows) { BLANK }
        private set
    var styles: ShortArray = ShortArray(cols * rows)
        private set
    var links: IntArray = IntArray(cols * rows)
        private set

    // Probe #223: what each cell wears on top of its code point. Sparse in practice and empty on
    // almost every cell, so it is a parallel array of references rather than a second IntArray:
    // a cluster does not fit in an Int and interning one would cost a table the wire does not send.
    var marks: Array<String> = Array(cols * rows) { "" }
        private set

    private var dirty = BooleanArray(rows) { true }

    fun resize(newCols: Int, newRows: Int) {
        if (newCols == cols && newRows == rows) return
        cols = newCols
        rows = newRows
        glyphs = IntArray(cols * rows) { BLANK }
        styles = ShortArray(cols * rows)
        links = IntArray(cols * rows)
        marks = Array(cols * rows) { "" }
        dirty = BooleanArray(rows) { true }
    }

    fun clear() {
        glyphs.fill(BLANK)
        styles.fill(0)
        links.fill(0)
        marks.fill("")
        dirty.fill(true)
    }

    fun isDirty(row: Int): Boolean = row in 0 until rows && dirty[row]

    fun clearDirty() = dirty.fill(false)

    fun apply(diff: RowDiff) {
        val row = diff.row
        if (row < 0 || row >= rows) return
        val base = row * cols
        var col = 0
        runs@ for (run in diff.runs) {
            val style = run.s.toShort()
            val link = run.l?.plus(1) ?: 0
            val width = if (run.w >= 2) 2 else 1
            var i = 0
            var cell = 0
            while (i < run.x.length) {
                val glyph = glyphAt(run.x, i)
                i += glyphUnits(glyph)
                if (col + width > cols) break@runs
                glyphs[base + col] = glyph
                styles[base + col] = style
                links[base + col] = link
                marks[base + col] = run.m.getOrElse(cell) { "" }
                if (width == 2) {
                    // The tail carries the lead's pen so a coloured background spans the whole
                    // glyph and a linked wide glyph is underlined across both its columns. It
                    // never carries the marks: those belong to the lead and would draw twice.
                    glyphs[base + col + 1] = TAIL
                    styles[base + col + 1] = style
                    links[base + col + 1] = link
                    marks[base + col + 1] = ""
                }
                col += width
                cell++
            }
        }
        while (col < cols) {
            glyphs[base + col] = BLANK
            styles[base + col] = 0
            links[base + col] = 0
            marks[base + col] = ""
            col++
        }
        dirty[row] = true
    }

    fun codePointAt(col: Int, row: Int): Int = glyphs[row * cols + col]

    fun styleAt(col: Int, row: Int): Int = styles[row * cols + col].toInt()

    fun linkAt(col: Int, row: Int): Int = links[row * cols + col] - 1

    fun marksAt(col: Int, row: Int): String = marks[row * cols + col]

    fun rowText(row: Int): String {
        val base = row * cols
        val builder = StringBuilder(cols)
        for (col in 0 until cols) {
            val glyph = glyphs[base + col]
            if (glyph != TAIL) builder.appendGlyph(glyph).append(marks[base + col])
        }
        return builder.toString().trimEnd()
    }
}
