package dev.kampr.terminal.render

import dev.kampr.shared.model.PaneState

// Scrollback and the live grid are one continuous surface addressed by a single index: history
// runs [0, historyRows), the live viewport follows it and is pinned to the bottom. There is no
// client-side cap — the node's ring bound is a memory limit, not a display one.
class SurfaceRows(private val pane: PaneState) {
    val cols: Int get() = pane.cells.cols
    val liveRows: Int get() = pane.cells.rows
    val historyRows: Int get() = pane.scrollback.historyRows
    val total: Int get() = historyRows + liveRows

    fun into(index: Int, chars: CharArray, styleIds: IntArray, linkIds: IntArray? = null): Boolean {
        val width = cols
        if (width == 0) return false
        val history = historyRows
        if (index < 0 || index >= history + liveRows) return false
        if (index >= history) {
            val row = index - history
            val base = row * width
            val cells = pane.cells
            for (col in 0 until width) {
                chars[col] = cells.chars[base + col]
                styleIds[col] = cells.styles[base + col].toInt()
                linkIds?.set(col, cells.links[base + col] - 1)
            }
            return true
        }
        val diff = pane.scrollback.row(pane.scrollback.fromTop + index)
        if (diff == null) {
            chars.fill(' ', 0, width)
            styleIds.fill(0, 0, width)
            linkIds?.fill(-1, 0, width)
            return true
        }
        var col = 0
        for (run in diff.runs) {
            val style = run.s
            val link = run.l ?: -1
            for (ch in run.x) {
                if (col >= width) break
                chars[col] = ch
                styleIds[col] = style
                linkIds?.set(col, link)
                col++
            }
            if (col >= width) break
        }
        while (col < width) {
            chars[col] = ' '
            styleIds[col] = 0
            linkIds?.set(col, -1)
            col++
        }
        return true
    }
}
