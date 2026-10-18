package dev.kampr.terminal.render

// A selection is cells, and a full-screen program scrolls its own text through the same cells —
// so a highlight left on its indices ends up over whatever scrolled under it. What it was laid on
// is kept beside it, and each repaint looks for that text again.
class SelectedText(val selection: Selection, private val rows: List<String>) {
    fun follow(rowAt: (Int) -> String, total: Int): SelectedText {
        val first = selection.start.row
        if (rows.all { it.isBlank() } || matchesAt(first, rowAt, total)) return this
        val offset = (1 until total).asSequence()
            .flatMap { sequenceOf(-it, it) }
            .firstOrNull { matchesAt(first + it, rowAt, total) }
            ?: return capture(selection, rowAt)
        return SelectedText(selection.shifted(offset), rows)
    }

    private fun matchesAt(row: Int, rowAt: (Int) -> String, total: Int): Boolean {
        if (row < 0 || row + rows.size > total) return false
        return rows.indices.all { rowAt(row + it) == rows[it] }
    }

    companion object {
        fun capture(selection: Selection, rowAt: (Int) -> String): SelectedText {
            val rows = (selection.start.row..selection.end.row).map(rowAt)
            return SelectedText(selection, rows)
        }
    }
}

private fun Selection.shifted(rows: Int): Selection = copy(
    anchor = anchor.copy(row = anchor.row + rows),
    head = head.copy(row = head.row + rows),
)
