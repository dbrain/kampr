package dev.kampr.terminal.spike

class CellBuffer(cols: Int, rows: Int) {
    var cols = cols
        private set
    var rows = rows
        private set
    var chars = CharArray(cols * rows) { ' ' }
        private set
    var styleIds = ShortArray(cols * rows)
        private set
    var dirty = BooleanArray(rows) { true }
        private set
    var cursor = CursorPos(0, 0, true)
        private set
    fun resize(newCols: Int, newRows: Int) {
        if (newCols == cols && newRows == rows) return
        cols = newCols
        rows = newRows
        chars = CharArray(cols * rows) { ' ' }
        styleIds = ShortArray(cols * rows)
        dirty = BooleanArray(rows) { true }
    }

    fun markAllDirty() = dirty.fill(true)

    fun clearDirty() = dirty.fill(false)

    fun apply(msg: GridReset) {
        resize(msg.cols, msg.rows)
        chars.fill(' ')
        styleIds.fill(0)
        for (rd in msg.rowsData) writeRow(rd)
        markAllDirty()
        cursor = msg.cursor
    }

    fun apply(msg: GridPatch) {
        for (rd in msg.rows) writeRow(rd)
        cursor = msg.cursor
    }

    private fun writeRow(rd: RowDiff) {
        val r = rd.row
        if (r < 0 || r >= rows) return
        val base = r * cols
        var col = 0
        for (run in rd.runs) {
            val sid = run.s.toShort()
            val text = run.x
            var i = 0
            while (i < text.length && col < cols) {
                chars[base + col] = text[i]
                styleIds[base + col] = sid
                col++
                i++
            }
            if (col >= cols) break
        }
        while (col < cols) {
            chars[base + col] = ' '
            styleIds[base + col] = 0
            col++
        }
        dirty[r] = true
    }
}
