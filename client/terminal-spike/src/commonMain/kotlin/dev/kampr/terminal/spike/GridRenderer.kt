package dev.kampr.terminal.spike

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.text.drawText
import kotlin.math.ceil

enum class RenderMode(val label: String) {
    NO_TEXT("backgrounds only"),
    SHAPE_EVERY_FRAME("shape every frame"),
    RUN_CACHE("cached run layouts"),
    GLYPH_CACHE("cached per-glyph layouts"),
    GLYPH_ATLAS("glyph atlas, tinted blits"),
    BITMAP_DIRTY("dirty rows into ImageBitmap"),
}

class GridRenderer(private val cache: TextCache) {
    private var bitmap: ImageBitmap? = null
    private var bmpW = 0
    private var bmpH = 0
    private val offscreen = CanvasDrawScope()
    private val sb = StringBuilder(512)
    private val atlas = GlyphAtlas(cache)

    val atlasGlyphs get() = atlas.rasterized

    fun invalidate() {
        bitmap = null
        bmpW = 0
        bmpH = 0
        atlas.invalidate()
    }

    fun render(
        ds: DrawScope,
        buf: CellBuffer,
        st: StyleTable,
        m: GridMetrics,
        mode: RenderMode,
        originX: Float,
        originY: Float,
        cursorOn: Boolean,
    ) {
        val gw = buf.cols * m.cellW
        val gh = buf.rows * m.cellH
        ds.drawRect(Color(Palette.DEFAULT_BG), Offset(originX, originY), Size(gw, gh))
        if (mode == RenderMode.GLYPH_ATLAS) atlas.prepare(ds, m, buf, st)
        if (mode == RenderMode.BITMAP_DIRTY) {
            renderViaBitmap(ds, buf, st, m, originX, originY)
        } else {
            for (r in 0 until buf.rows) drawRow(ds, buf, st, m, r, originX, originY, mode)
        }
        drawCursor(ds, buf, st, m, originX, originY, cursorOn)
        buf.clearDirty()
    }

    private fun renderViaBitmap(
        ds: DrawScope,
        buf: CellBuffer,
        st: StyleTable,
        m: GridMetrics,
        originX: Float,
        originY: Float,
    ) {
        val w = ceil(buf.cols * m.cellW).toInt().coerceAtLeast(1)
        val h = ceil(buf.rows * m.cellH).toInt().coerceAtLeast(1)
        var bmp = bitmap
        if (bmp == null || bmpW != w || bmpH != h) {
            bmp = ImageBitmap(w, h)
            bitmap = bmp
            bmpW = w
            bmpH = h
            buf.markAllDirty()
        }
        val anyDirty = buf.dirty.any { it }
        if (anyDirty) {
            offscreen.draw(ds, ds.layoutDirection, Canvas(bmp), Size(w.toFloat(), h.toFloat())) {
                for (r in 0 until buf.rows) {
                    if (!buf.dirty[r]) continue
                    drawRect(
                        Color(Palette.DEFAULT_BG),
                        Offset(0f, r * m.cellH),
                        Size(w.toFloat(), m.cellH),
                    )
                    drawRow(this, buf, st, m, r, 0f, 0f, RenderMode.RUN_CACHE)
                }
            }
        }
        ds.drawImage(bmp, topLeft = Offset(originX, originY))
    }

    private fun drawRow(
        ds: DrawScope,
        buf: CellBuffer,
        st: StyleTable,
        m: GridMetrics,
        row: Int,
        ox: Float,
        oy: Float,
        mode: RenderMode,
    ) {
        val cols = buf.cols
        val base = row * cols
        val chars = buf.chars
        val ids = buf.styleIds
        val y = oy + row * m.cellH

        var c = 0
        while (c < cols) {
            val bg = st.bg[ids[base + c].toInt()]
            var e = c + 1
            while (e < cols && st.bg[ids[base + e].toInt()] == bg) e++
            if (bg != Palette.DEFAULT_BG) {
                ds.drawRect(
                    Color(bg),
                    Offset(ox + c * m.cellW, y),
                    Size((e - c) * m.cellW, m.cellH),
                )
            }
            c = e
        }

        if (mode == RenderMode.NO_TEXT) return

        if (mode == RenderMode.GLYPH_ATLAS) {
            val yi = (y + 0.5f).toInt()
            for (i in 0 until cols) {
                val ch = chars[base + i]
                if (ch == ' ') continue
                val sid = ids[base + i].toInt()
                val fk = st.fontKey[sid]
                val xf = ox + i * m.cellW
                if (!atlas.blit(ds, ch, fk, st.fg[sid], (xf + 0.5f).toInt(), yi)) {
                    ds.drawText(cache.glyph(ch, fk), Color(st.fg[sid]), Offset(xf, y))
                }
            }
            drawDecorations(ds, buf, st, m, row, ox, y)
            return
        }

        if (mode == RenderMode.GLYPH_CACHE) {
            for (i in 0 until cols) {
                val ch = chars[base + i]
                if (ch == ' ') continue
                val sid = ids[base + i].toInt()
                ds.drawText(
                    cache.glyph(ch, st.fontKey[sid]),
                    Color(st.fg[sid]),
                    Offset(ox + i * m.cellW, y),
                )
            }
            return
        }

        c = 0
        while (c < cols) {
            val sid = ids[base + c].toInt()
            val fg = st.fg[sid]
            val fk = st.fontKey[sid]
            var e = c + 1
            while (e < cols) {
                val s2 = ids[base + e].toInt()
                if (st.fg[s2] != fg || st.fontKey[s2] != fk) break
                e++
            }
            var end = e
            if (fk and (FONT_UNDERLINE or FONT_STRIKE) == 0) {
                while (end > c && chars[base + end - 1] == ' ') end--
            }
            if (end > c) {
                sb.setLength(0)
                for (i in c until end) sb.append(chars[base + i])
                val text = sb.toString()
                val layout =
                    if (mode == RenderMode.RUN_CACHE) cache.cachedRun(text, fk)
                    else cache.shape(text, fk)
                ds.drawText(layout, Color(fg), Offset(ox + c * m.cellW, y))
            }
            c = e
        }
    }

    private fun drawDecorations(
        ds: DrawScope,
        buf: CellBuffer,
        st: StyleTable,
        m: GridMetrics,
        row: Int,
        ox: Float,
        y: Float,
    ) {
        val base = row * buf.cols
        val thickness = (m.cellH * 0.06f).coerceAtLeast(1f)
        var c = 0
        while (c < buf.cols) {
            val fk = st.fontKey[buf.styleIds[base + c].toInt()]
            if (fk and (FONT_UNDERLINE or FONT_STRIKE) == 0) {
                c++
                continue
            }
            val fg = st.fg[buf.styleIds[base + c].toInt()]
            var e = c + 1
            while (e < buf.cols) {
                val s2 = buf.styleIds[base + e].toInt()
                if (st.fontKey[s2] != fk || st.fg[s2] != fg) break
                e++
            }
            val w = (e - c) * m.cellW
            if (fk and FONT_UNDERLINE != 0) {
                ds.drawRect(Color(fg), Offset(ox + c * m.cellW, y + m.cellH * 0.88f), Size(w, thickness))
            }
            if (fk and FONT_STRIKE != 0) {
                ds.drawRect(Color(fg), Offset(ox + c * m.cellW, y + m.cellH * 0.52f), Size(w, thickness))
            }
            c = e
        }
    }

    private fun drawCursor(
        ds: DrawScope,
        buf: CellBuffer,
        st: StyleTable,
        m: GridMetrics,
        ox: Float,
        oy: Float,
        on: Boolean,
    ) {
        if (!on || !buf.cursor.visible) return
        val cc = buf.cursor.col.coerceIn(0, buf.cols - 1)
        val cr = buf.cursor.row.coerceIn(0, buf.rows - 1)
        val idx = cr * buf.cols + cc
        val sid = buf.styleIds[idx].toInt()
        val x = ox + cc * m.cellW
        val y = oy + cr * m.cellH
        ds.drawRect(Color(st.fg[sid]), Offset(x, y), Size(m.cellW, m.cellH))
        val ch = buf.chars[idx]
        if (ch != ' ') {
            ds.drawText(cache.glyph(ch, st.fontKey[sid]), Color(st.bg[sid]), Offset(x, y))
        }
    }
}
