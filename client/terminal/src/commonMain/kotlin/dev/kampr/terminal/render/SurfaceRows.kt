package dev.kampr.terminal.render

import dev.kampr.shared.model.BLANK
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.model.TAIL
import dev.kampr.shared.model.glyphAt
import dev.kampr.shared.model.glyphUnits

// Scrollback and the live grid are one continuous surface addressed by a single index: history
// runs [0, historyRows), the live viewport follows it and is pinned to the bottom. There is no
// client-side cap — the node's ring bound is a memory limit, not a display one.
class SurfaceRows(private val pane: PaneState) {
    val cols: Int get() = pane.cells.cols
    val liveRows: Int get() = pane.cells.rows
    val historyRows: Int get() = pane.scrollback.historyRows
    val total: Int get() = historyRows + liveRows

    // The ring's own coordinates, not the surface's. History rows keep their absolute index
    // when more history arrives, which is what a reader parked on one can be anchored to.
    val fromTop: Int get() = pane.scrollback.fromTop
    val capped: Boolean get() = pane.scrollback.capped
    val complete: Boolean get() = pane.scrollback.complete

    // One code point per column, with TAIL where a column is the right half of the double-width
    // glyph beside it, and `marks` carrying whatever each cell wears on top of that code point.
    // Scrollback rows are decoded from their runs the same way the live buffer decodes them, so
    // the two halves of the surface read alike.
    fun into(
        index: Int,
        glyphs: IntArray,
        styleIds: IntArray,
        linkIds: IntArray? = null,
        marks: Array<String>? = null,
    ): Boolean {
        val width = cols
        if (width == 0) return false
        val history = historyRows
        if (index < 0 || index >= history + liveRows) return false
        if (index >= history) {
            val row = index - history
            val base = row * width
            val cells = pane.cells
            for (col in 0 until width) {
                glyphs[col] = cells.glyphs[base + col]
                styleIds[col] = cells.styles[base + col].toInt()
                linkIds?.set(col, cells.links[base + col] - 1)
                marks?.set(col, cells.marks[base + col])
            }
            return true
        }
        val diff = pane.scrollback.row(pane.scrollback.fromTop + index)
        if (diff == null) {
            glyphs.fill(BLANK, 0, width)
            styleIds.fill(0, 0, width)
            linkIds?.fill(-1, 0, width)
            marks?.fill("", 0, width)
            return true
        }
        var col = 0
        runs@ for (run in diff.runs) {
            val style = run.s
            val link = run.l ?: -1
            val glyphWidth = if (run.w >= 2) 2 else 1
            var i = 0
            var cell = 0
            while (i < run.x.length) {
                val glyph = glyphAt(run.x, i)
                i += glyphUnits(glyph)
                if (col + glyphWidth > width) break@runs
                glyphs[col] = glyph
                styleIds[col] = style
                linkIds?.set(col, link)
                marks?.set(col, run.m.getOrElse(cell) { "" })
                if (glyphWidth == 2) {
                    glyphs[col + 1] = TAIL
                    styleIds[col + 1] = style
                    linkIds?.set(col + 1, link)
                    marks?.set(col + 1, "")
                }
                col += glyphWidth
                cell++
            }
        }
        while (col < width) {
            glyphs[col] = BLANK
            styleIds[col] = 0
            linkIds?.set(col, -1)
            marks?.set(col, "")
            col++
        }
        return true
    }
}
