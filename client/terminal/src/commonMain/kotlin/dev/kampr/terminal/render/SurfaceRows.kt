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

    // How far the end of what there is to read sits from the end of the surface, counted the way
    // the caret is — the row itself included, so a pane whose last row is written answers 1 and a
    // pane with nothing below its caret answers what the caret answers.
    //
    // A herdr pane is as tall as the desk made it and the shell fills as much of it as it likes,
    // so the grid's last row and the content's last row are two different rows on a pane that has
    // just been cleared, or has four lines in a ninety-row window. Everything below the second is
    // blank tail: it is drawn, because the surface paints the whole viewport, but there is nothing
    // in it to travel to.
    //
    // **The caret is content**, which is what makes this a live-grid scan and never a history one:
    // a cleared shell is a prompt and a place to type, both on a live row, and the caret cannot be
    // in history — so the answer is always a row at or below it and the walk stops there. That
    // bounds the cost by the blank tail itself, which is the only case that has any: a pane whose
    // last row is written costs one row of comparisons, and a pane with none costs zero.
    //
    // Blank is the glyph *and* the pen: a full-width run of spaces in a program's own colours is a
    // status bar, not an empty row, and stopping above one would put it out of reach.
    fun contentBelow(cursorRow: Int): Int {
        val width = cols
        val live = liveRows
        val caret = cursorRow.coerceIn(0, (live - 1).coerceAtLeast(0))
        if (width == 0) return live - caret
        val cells = pane.cells
        var row = live - 1
        while (row > caret) {
            val base = row * width
            for (cell in base until base + width) {
                if (cells.glyphs[cell] != BLANK || cells.styles[cell].toInt() != 0) return live - row
            }
            row--
        }
        return live - caret
    }

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
