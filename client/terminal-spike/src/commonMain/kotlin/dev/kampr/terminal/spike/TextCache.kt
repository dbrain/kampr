package dev.kampr.terminal.spike

import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextMeasurer
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.sp

class GridMetrics(
    val fontSizePx: Float,
    val cellW: Float,
    val cellH: Float,
    val textTop: Float,
)

class TextCache(
    private val measurer: TextMeasurer,
    private val family: FontFamily,
) {
    private var fontSize: TextUnit = 0.sp
    private val styles = arrayOfNulls<TextStyle>(16)
    private val glyphs = arrayOfNulls<TextLayoutResult>(128 * 16)
    private val wideGlyphs = HashMap<Int, TextLayoutResult>()
    private val runs = HashMap<RunKey, TextLayoutResult>()

    var runCacheHits = 0
        private set
    var runCacheMisses = 0
        private set

    val runCacheSize get() = runs.size

    fun ensureSize(sizeSp: TextUnit) {
        if (sizeSp == fontSize) return
        fontSize = sizeSp
        styles.fill(null)
        glyphs.fill(null)
        wideGlyphs.clear()
        runs.clear()
    }

    fun resetCounters() {
        runCacheHits = 0
        runCacheMisses = 0
    }

    fun style(fontKey: Int): TextStyle = styles[fontKey] ?: TextStyle(
        fontFamily = family,
        fontSize = fontSize,
        fontWeight = if (fontKey and FONT_BOLD != 0) FontWeight.Bold else FontWeight.Normal,
        fontStyle = if (fontKey and FONT_ITALIC != 0) FontStyle.Italic else FontStyle.Normal,
        textDecoration = decoration(fontKey),
    ).also { styles[fontKey] = it }

    private fun decoration(fontKey: Int): TextDecoration? {
        val u = fontKey and FONT_UNDERLINE != 0
        val s = fontKey and FONT_STRIKE != 0
        return when {
            u && s -> TextDecoration.combine(listOf(TextDecoration.Underline, TextDecoration.LineThrough))
            u -> TextDecoration.Underline
            s -> TextDecoration.LineThrough
            else -> null
        }
    }

    fun shape(text: String, fontKey: Int): TextLayoutResult =
        measurer.measure(text, style(fontKey), softWrap = false, maxLines = 1, skipCache = true)

    fun cachedRun(text: String, fontKey: Int): TextLayoutResult {
        val key = RunKey(text, fontKey)
        val hit = runs[key]
        if (hit != null) {
            runCacheHits++
            return hit
        }
        runCacheMisses++
        if (runs.size > RUN_CACHE_LIMIT) runs.clear()
        val r = shape(text, fontKey)
        runs[key] = r
        return r
    }

    fun glyph(ch: Char, fontKey: Int): TextLayoutResult {
        val code = ch.code
        if (code < 128) {
            val idx = code * 16 + fontKey
            return glyphs[idx] ?: shape(ch.toString(), fontKey).also { glyphs[idx] = it }
        }
        val k = code * 16 + fontKey
        return wideGlyphs.getOrPut(k) { shape(ch.toString(), fontKey) }
    }

    private var metricsCache: GridMetrics? = null
    private var metricsSize: TextUnit = 0.sp

    fun revalidate(sizeSp: TextUnit): Boolean {
        val prev = metricsCache
        val m = metrics(sizeSp)
        metricsCache = m
        metricsSize = sizeSp
        if (prev != null && kotlin.math.abs(prev.cellW - m.cellW) < 0.01f) return false
        styles.fill(null)
        glyphs.fill(null)
        wideGlyphs.clear()
        runs.clear()
        return true
    }

    fun metricsFor(sizeSp: TextUnit): GridMetrics {
        val c = metricsCache
        if (c != null && metricsSize == sizeSp) return c
        val m = metrics(sizeSp)
        metricsCache = m
        metricsSize = sizeSp
        return m
    }

    fun metrics(sizeSp: TextUnit): GridMetrics {
        ensureSize(sizeSp)
        val probe = measurer.measure(
            PROBE, style(0), softWrap = false, maxLines = 1,
            constraints = Constraints(), skipCache = true,
        )
        val cellW = probe.size.width.toFloat() / PROBE.length
        val lineH = probe.size.height.toFloat()
        val cellH = lineH
        return GridMetrics(
            fontSizePx = 0f,
            cellW = cellW,
            cellH = cellH,
            textTop = 0f,
        )
    }

    private companion object {
        const val RUN_CACHE_LIMIT = 4096
        const val PROBE = "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM"
    }
}

data class RunKey(val text: String, val fontKey: Int)
