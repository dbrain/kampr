package dev.kampr.terminal.render

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.text.drawText
import kotlin.math.ceil
import kotlin.math.floor

class GridRenderer(private val cache: TextCache, private val modes: ModeSelector) {
    private var chars = CharArray(0)
    private var styleIds = IntArray(0)
    private var linkIds = IntArray(0)
    private val builder = StringBuilder(256)

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
        val cols = rows.cols
        val total = rows.total
        scope.drawRect(Color(styles.defaultBg), Offset.Zero, scope.size)
        if (cols == 0 || total == 0 || cellWidth <= 0f || cellHeight <= 0f) return
        if (chars.size < cols) {
            chars = CharArray(cols)
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
            if (!rows.into(index, chars, styleIds, linkIds)) continue
            val y = originY + index * cellHeight
            paintBackgrounds(scope, styles, firstCol, lastCol, originX, y, cellWidth, cellHeight)
            selection?.span(index, cols)?.let { span ->
                val from = maxOf(span.first, firstCol)
                val to = minOf(span.last, lastCol - 1)
                if (to >= from) {
                    scope.drawRect(
                        selectionWash,
                        Offset(originX + from * cellWidth, y),
                        Size((to - from + 1) * cellWidth, cellHeight),
                    )
                }
            }
            when (modes.mode) {
                RenderMode.CachedRuns -> paintRuns(scope, styles, firstCol, lastCol, originX, y, cellWidth)
                RenderMode.PerGlyph -> paintGlyphs(scope, styles, firstCol, lastCol, originX, y, cellWidth)
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
            var trimmed = end
            if (key and (FONT_UNDERLINE or FONT_STRIKE) == 0) {
                while (trimmed > col && chars[trimmed - 1] == ' ') trimmed--
            }
            if (trimmed > col) {
                builder.setLength(0)
                for (i in col until trimmed) builder.append(chars[i])
                val text = builder.toString()
                modes.observeRunKey(text.hashCode() * 31 + key)
                scope.drawText(cache.run(text, key), Color(fg), Offset(originX + col * cellWidth, y))
            }
            col = end
        }
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
            for (i in col until end) hash = hash * 31 + chars[i].code
            modes.observeRunKey(hash * 31 + key)
            for (i in col until end) {
                val ch = chars[i]
                if (ch == ' ' && key and (FONT_UNDERLINE or FONT_STRIKE) == 0) continue
                scope.drawText(
                    cache.glyph(ch, key),
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
        if (!rows.into(index, chars, styleIds)) return
        val id = styles.clamp(styleIds[col])
        val x = originX + col * cellWidth
        val y = originY + index * cellHeight
        scope.drawRect(Color(styles.fg[id]), Offset(x, y), Size(cellWidth, cellHeight))
        val ch = chars[col]
        if (ch != ' ') {
            scope.drawText(cache.glyph(ch, styles.fontKey[id]), Color(styles.bg[id]), Offset(x, y))
        }
    }
}
