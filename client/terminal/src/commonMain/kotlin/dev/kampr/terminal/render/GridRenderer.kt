package dev.kampr.terminal.render

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.text.drawText
import dev.kampr.shared.model.BLANK
import dev.kampr.shared.model.TAIL
import dev.kampr.shared.model.appendGlyph
import kotlin.math.ceil
import kotlin.math.floor

class GridRenderer(private val cache: TextCache, private val modes: ModeSelector) {
    private var glyphs = IntArray(0)
    private var styleIds = IntArray(0)
    private var linkIds = IntArray(0)
    private val builder = StringBuilder(256)
    private var cols = 0

    val mode: RenderMode get() = modes.mode

    fun reset() = modes.reset()

    // The paint rectangle is the whole viewport: rows run under the header and the key row so
    // nothing is ever blank. What insets is the scrollable content, which the caller expresses
    // through originY.
    fun draw(
        scope: DrawScope,
        rows: SurfaceRows,
        styles: ResolvedStyles,
        cellWidth: Float,
        cellHeight: Float,
        originX: Float,
        originY: Float,
        cursorCol: Int,
        cursorRow: Int,
        cursorOn: Boolean,
        selection: Selection?,
        selectionWash: Color,
        linkInk: Color,
    ) {
        cols = rows.cols
        val total = rows.total
        scope.drawRect(Color(styles.defaultBg), Offset.Zero, scope.size)
        if (cols == 0 || total == 0 || cellWidth <= 0f || cellHeight <= 0f) return
        if (glyphs.size < cols) {
            glyphs = IntArray(cols)
            styleIds = IntArray(cols)
            linkIds = IntArray(cols)
        }

        val firstRow = floor(-originY / cellHeight).toInt().coerceIn(0, total)
        val lastRow = ceil((scope.size.height - originY) / cellHeight).toInt().coerceIn(0, total)
        val firstCol = floor(-originX / cellWidth).toInt().coerceIn(0, cols)
        val lastCol = ceil((scope.size.width - originX) / cellWidth).toInt().coerceIn(0, cols)
        if (firstCol >= lastCol) return

        cache.resetCounters()
        for (index in firstRow until lastRow) {
            if (!rows.into(index, glyphs, styleIds, linkIds)) continue
            val y = originY + index * cellHeight
            paintBackgrounds(scope, styles, firstCol, lastCol, originX, y, cellWidth, cellHeight)
            selection?.span(index, cols)?.let { span ->
                // A wash that stops between the halves of a glyph is a selection half a column
                // out, so an endpoint that lands on a tail takes its lead with it and back.
                var edge = span.first
                if (edge > 0 && glyphs[edge] == TAIL) edge--
                val from = maxOf(edge, firstCol)
                val to = minOf(if (wide(span.last)) span.last + 1 else span.last, lastCol - 1)
                if (to >= from) {
                    scope.drawRect(
                        selectionWash,
                        Offset(originX + from * cellWidth, y),
                        Size((to - from + 1) * cellWidth, cellHeight),
                    )
                }
            }
            // A wide glyph whose lead is off the left edge still has to be drawn or its visible
            // half disappears, so the ink passes start one column earlier than the paint window.
            val inkCol = if (firstCol > 0 && glyphs[firstCol] == TAIL) firstCol - 1 else firstCol
            when (modes.mode) {
                RenderMode.CachedRuns -> paintRuns(scope, styles, inkCol, lastCol, originX, y, cellWidth)
                RenderMode.PerGlyph -> paintGlyphs(scope, styles, inkCol, lastCol, originX, y, cellWidth)
            }
            paintLinks(scope, firstCol, lastCol, originX, y, cellWidth, cellHeight, linkInk)
        }
        modes.endFrame(cache.hitRate)

        if (cursorOn) {
            val index = rows.historyRows + cursorRow
            if (index in firstRow until lastRow && cursorCol in firstCol until lastCol) {
                paintCursor(scope, rows, styles, index, cursorCol, originX, originY, cellWidth, cellHeight)
            }
        }
    }

    // Probe #37: OSC 8 hyperlinks survive the frame stream, so a linked cell carries a real
    // harness-declared URI. Underlining it is how that becomes visible as something to tap.
    private fun paintLinks(
        scope: DrawScope,
        firstCol: Int,
        lastCol: Int,
        originX: Float,
        y: Float,
        cellWidth: Float,
        cellHeight: Float,
        ink: Color,
    ) {
        if (ink.alpha <= 0f) return
        var col = firstCol
        while (col < lastCol) {
            if (linkIds[col] < 0) {
                col++
                continue
            }
            var end = col + 1
            while (end < lastCol && linkIds[end] == linkIds[col]) end++
            scope.drawRect(
                ink,
                Offset(originX + col * cellWidth, y + cellHeight * 0.92f),
                Size((end - col) * cellWidth, (cellHeight * 0.06f).coerceAtLeast(1f)),
            )
            col = end
        }
    }

    private fun paintBackgrounds(
        scope: DrawScope,
        styles: ResolvedStyles,
        firstCol: Int,
        lastCol: Int,
        originX: Float,
        y: Float,
        cellWidth: Float,
        cellHeight: Float,
    ) {
        val default = styles.defaultBg
        var col = firstCol
        while (col < lastCol) {
            val bg = styles.bg[styles.clamp(styleIds[col])]
            var end = col + 1
            while (end < lastCol && styles.bg[styles.clamp(styleIds[end])] == bg) end++
            if (bg != default) {
                scope.drawRect(
                    Color(bg),
                    Offset(originX + col * cellWidth, y),
                    Size((end - col) * cellWidth, cellHeight),
                )
            }
            col = end
        }
    }

    private fun wide(col: Int): Boolean = col + 1 < cols && glyphs[col + 1] == TAIL

    // Probe #210: a run string is laid out at the font's own advances, and nothing guarantees the
    // fallback face that draws a CJK glyph or an emoji advances exactly two cells. So a cached run
    // covers narrow cells only and each wide glyph is drawn at the column it belongs to — the
    // fixed pitch is the contract, not the font.
    private fun paintRuns(
        scope: DrawScope,
        styles: ResolvedStyles,
        firstCol: Int,
        lastCol: Int,
        originX: Float,
        y: Float,
        cellWidth: Float,
    ) {
        var col = firstCol
        while (col < lastCol) {
            val id = styles.clamp(styleIds[col])
            val fg = styles.fg[id]
            val key = styles.fontKey[id]
            var end = col + 1
            while (end < lastCol) {
                val next = styles.clamp(styleIds[end])
                if (styles.fg[next] != fg || styles.fontKey[next] != key) break
                end++
            }
            var at = col
            while (at < end) {
                var stop = at
                while (stop < end && !wide(stop)) stop++
                var trimmed = stop
                if (key and (FONT_UNDERLINE or FONT_STRIKE) == 0) {
                    while (trimmed > at && glyphs[trimmed - 1] == BLANK) trimmed--
                }
                if (trimmed > at) {
                    builder.setLength(0)
                    for (i in at until trimmed) if (glyphs[i] != TAIL) builder.appendGlyph(glyphs[i])
                    val text = builder.toString()
                    modes.observeRunKey(text.hashCode() * 31 + key)
                    scope.drawText(cache.run(text, key), Color(fg), Offset(originX + at * cellWidth, y))
                }
                if (stop < end) {
                    drawWide(scope, glyphs[stop], key, fg, originX + stop * cellWidth, y, cellWidth)
                    at = stop + 2
                } else {
                    at = stop
                }
            }
            col = end
        }
    }

    // A fallback face drawing a CJK glyph or an emoji at the terminal font's em size rarely
    // advances exactly two cells, and the leftover shows as a gap. Centring it in the pair it owns
    // is what a terminal does with a glyph whose advance and whose cell disagree.
    private fun drawWide(
        scope: DrawScope,
        glyph: Int,
        fontKey: Int,
        fg: Int,
        x: Float,
        y: Float,
        cellWidth: Float,
    ) {
        val layout = cache.glyph(glyph, fontKey)
        val slack = (2f * cellWidth - layout.size.width) / 2f
        scope.drawText(layout, Color(fg), Offset(x + slack, y))
    }

    // Skia keeps its own GPU glyph atlas; one drawText per cell is how common code reaches it
    // (probe #61 — a hand-rolled atlas through drawImage aborts on wasm and is 2.2 fps on desktop).
    private fun paintGlyphs(
        scope: DrawScope,
        styles: ResolvedStyles,
        firstCol: Int,
        lastCol: Int,
        originX: Float,
        y: Float,
        cellWidth: Float,
    ) {
        var col = firstCol
        while (col < lastCol) {
            val id = styles.clamp(styleIds[col])
            val key = styles.fontKey[id]
            var end = col + 1
            while (end < lastCol) {
                val next = styles.clamp(styleIds[end])
                if (styles.fg[next] != styles.fg[id] || styles.fontKey[next] != key) break
                end++
            }
            var hash = 0
            for (i in col until end) hash = hash * 31 + glyphs[i]
            modes.observeRunKey(hash * 31 + key)
            for (i in col until end) {
                val glyph = glyphs[i]
                if (glyph == TAIL) continue
                if (glyph == BLANK && key and (FONT_UNDERLINE or FONT_STRIKE) == 0) continue
                if (wide(i)) {
                    drawWide(scope, glyph, key, styles.fg[id], originX + i * cellWidth, y, cellWidth)
                    continue
                }
                scope.drawText(
                    cache.glyph(glyph, key),
                    Color(styles.fg[id]),
                    Offset(originX + i * cellWidth, y),
                )
            }
            col = end
        }
    }

    private fun paintCursor(
        scope: DrawScope,
        rows: SurfaceRows,
        styles: ResolvedStyles,
        index: Int,
        col: Int,
        originX: Float,
        originY: Float,
        cellWidth: Float,
        cellHeight: Float,
    ) {
        if (!rows.into(index, glyphs, styleIds)) return
        // The caret sits on a glyph, not on half of one: landing on a tail column means the block
        // covers the pair and the glyph is drawn once, from its lead.
        val lead = if (col > 0 && glyphs[col] == TAIL) col - 1 else col
        val span = if (wide(lead)) 2 else 1
        val id = styles.clamp(styleIds[lead])
        val x = originX + lead * cellWidth
        val y = originY + index * cellHeight
        scope.drawRect(Color(styles.fg[id]), Offset(x, y), Size(cellWidth * span, cellHeight))
        val glyph = glyphs[lead]
        if (glyph == BLANK) return
        if (span == 2) {
            drawWide(scope, glyph, styles.fontKey[id], styles.bg[id], x, y, cellWidth)
        } else {
            scope.drawText(cache.glyph(glyph, styles.fontKey[id]), Color(styles.bg[id]), Offset(x, y))
        }
    }
}
