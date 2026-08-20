package dev.kampr.terminal.spike

import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Canvas
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ColorFilter
import androidx.compose.ui.graphics.FilterQuality
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.drawscope.CanvasDrawScope
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.text.drawText
import androidx.compose.ui.unit.Density
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import kotlin.math.ceil

private const val ATLAS_COLS = 32
private const val ATLAS_CAPACITY = 1024

class GlyphAtlas(private val cache: TextCache) {
    private var image: ImageBitmap? = null
    private var cw = 0
    private var ch = 0
    private var next = 0
    private var builtFor = -1f
    private val slots = HashMap<Int, Int>()
    private val pending = ArrayList<Int>()
    private val tints = HashMap<Int, ColorFilter>()
    private val scratch = CanvasDrawScope()

    fun invalidate() {
        image = null
        slots.clear()
        pending.clear()
        next = 0
        builtFor = -1f
    }

    fun prepare(density: Density, m: GridMetrics, buf: CellBuffer, st: StyleTable) {
        if (builtFor != m.cellW || image == null) {
            slots.clear()
            next = 0
            cw = ceil(m.cellW).toInt().coerceAtLeast(1)
            ch = ceil(m.cellH).toInt().coerceAtLeast(1)
            val rows = (ATLAS_CAPACITY + ATLAS_COLS - 1) / ATLAS_COLS
            image = ImageBitmap(ATLAS_COLS * cw, rows * ch)
            builtFor = m.cellW
        }
        pending.clear()
        val chars = buf.chars
        val ids = buf.styleIds
        for (i in chars.indices) {
            val c = chars[i]
            if (c == ' ') continue
            val key = c.code * 4 + (st.fontKey[ids[i].toInt()] and (FONT_BOLD or FONT_ITALIC))
            if (slots.containsKey(key)) continue
            if (next >= ATLAS_CAPACITY) continue
            slots[key] = next
            pending.add(key)
            next++
        }
        if (pending.isEmpty()) return
        val bmp = image ?: return
        scratch.draw(
            density,
            androidx.compose.ui.unit.LayoutDirection.Ltr,
            Canvas(bmp),
            Size(bmp.width.toFloat(), bmp.height.toFloat()),
        ) {
            for (key in pending) {
                val slot = slots[key]!!
                val sx = (slot % ATLAS_COLS) * cw
                val sy = (slot / ATLAS_COLS) * ch
                drawRect(
                    Color.Transparent,
                    Offset(sx.toFloat(), sy.toFloat()),
                    Size(cw.toFloat(), ch.toFloat()),
                    blendMode = BlendMode.Clear,
                )
                drawText(
                    cache.glyph((key / 4).toChar(), key % 4),
                    Color.White,
                    Offset(sx.toFloat(), sy.toFloat()),
                )
            }
        }
        pending.clear()
    }

    fun blit(ds: DrawScope, ch0: Char, fontKey: Int, fg: Int, x: Int, y: Int): Boolean {
        val bmp = image ?: return false
        val slot = slots[ch0.code * 4 + (fontKey and (FONT_BOLD or FONT_ITALIC))] ?: return false
        val sx = (slot % ATLAS_COLS) * cw
        val sy = (slot / ATLAS_COLS) * ch
        ds.drawImage(
            image = bmp,
            srcOffset = IntOffset(sx, sy),
            srcSize = IntSize(cw, ch),
            dstOffset = IntOffset(x, y),
            dstSize = IntSize(cw, ch),
            colorFilter = tints.getOrPut(fg) { ColorFilter.tint(Color(fg), BlendMode.SrcIn) },
            filterQuality = FilterQuality.None,
        )
        return true
    }
}
