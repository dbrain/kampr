package dev.kampr.terminal.render

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
import dev.kampr.shared.model.glyphString

data class CellMetrics(val width: Float, val height: Float)

// Probe #58/#59: shaping is 93% of frame cost and run strings repeat 99.2% of the time, so the
// run-layout cache is load-bearing, not an optimisation. Colour is applied at draw time, so one
// layout serves every pen that shares its text and font key.
class TextCache(private val measurer: TextMeasurer, private val family: FontFamily) {
    private var size: TextUnit = 0.sp
    private val styles = arrayOfNulls<TextStyle>(16)
    private val asciiGlyphs = arrayOfNulls<TextLayoutResult>(128 * 16)
    private val wideGlyphs = HashMap<Int, TextLayoutResult>()
    private val runs = HashMap<RunKey, TextLayoutResult>()
    private val clusters = HashMap<RunKey, TextLayoutResult>()
    private val probeKey = RunKey("", 0)

    var hits = 0
        private set
    var misses = 0
        private set

    val hitRate: Float get() = if (hits + misses == 0) 1f else hits.toFloat() / (hits + misses)

    fun resetCounters() {
        hits = 0
        misses = 0
    }

    private var advance = 0f

    fun sizeFor(value: TextUnit) {
        if (value == size) return
        size = value
        invalidate()
    }

    private fun invalidate() {
        styles.fill(null)
        asciiGlyphs.fill(null)
        wideGlyphs.clear()
        runs.clear()
        clusters.clear()
    }

    // Probe #65: a resource font resolves asynchronously and Android does not gate first layout on
    // it, so an advance measured once can belong to a font that never gets drawn. Re-probing until
    // it stops moving is the only thing that catches it.
    fun reprobe(value: TextUnit): Boolean {
        val previous = advance
        val current = metrics(value).width
        if (previous <= 0f || kotlin.math.abs(current - previous) < 0.01f) return false
        invalidate()
        return true
    }

    fun style(fontKey: Int): TextStyle = styles[fontKey] ?: TextStyle(
        fontFamily = family,
        fontSize = size,
        fontWeight = if (fontKey and FONT_BOLD != 0) FontWeight.W700 else FontWeight.W400,
        fontStyle = if (fontKey and FONT_ITALIC != 0) FontStyle.Italic else FontStyle.Normal,
        textDecoration = decoration(fontKey),
    ).also { styles[fontKey] = it }

    fun shape(text: String, fontKey: Int): TextLayoutResult =
        measurer.measure(text, style(fontKey), softWrap = false, maxLines = 1, skipCache = true)

    fun run(text: String, fontKey: Int): TextLayoutResult {
        probeKey.text = text
        probeKey.fontKey = fontKey
        val hit = runs[probeKey]
        if (hit != null) {
            hits++
            return hit
        }
        misses++
        if (runs.size > RUN_LIMIT) runs.clear()
        return shape(text, fontKey).also { runs[RunKey(text, fontKey)] = it }
    }

    // A cell wearing marks is one grapheme drawn on its own, so it is cached by its whole text and
    // kept out of the run counters the mode selector reads: it is never a run.
    fun cluster(text: String, fontKey: Int): TextLayoutResult =
        clusters.getOrPut(RunKey(text, fontKey)) { shape(text, fontKey) }

    fun glyph(codePoint: Int, fontKey: Int): TextLayoutResult {
        if (codePoint < 128) {
            val index = codePoint * 16 + fontKey
            return asciiGlyphs[index] ?: shape(glyphString(codePoint), fontKey).also { asciiGlyphs[index] = it }
        }
        return wideGlyphs.getOrPut(codePoint * 16 + fontKey) { shape(glyphString(codePoint), fontKey) }
    }

    // Probe #65: a resource font resolves asynchronously, so the advance is re-probed whenever the
    // family or the size changes rather than measured once against whatever was loaded first.
    fun metrics(value: TextUnit): CellMetrics {
        sizeFor(value)
        val probe = measurer.measure(
            PROBE, style(0), softWrap = false, maxLines = 1,
            constraints = Constraints(), skipCache = true,
        )
        advance = probe.size.width.toFloat() / PROBE.length
        return CellMetrics(advance, probe.size.height.toFloat())
    }

    private fun decoration(fontKey: Int): TextDecoration? {
        val underline = fontKey and FONT_UNDERLINE != 0
        val strike = fontKey and FONT_STRIKE != 0
        return when {
            underline && strike -> TextDecoration.combine(
                listOf(TextDecoration.Underline, TextDecoration.LineThrough),
            )
            underline -> TextDecoration.Underline
            strike -> TextDecoration.LineThrough
            else -> null
        }
    }

    private companion object {
        const val RUN_LIMIT = 4096
        const val PROBE = "MMMMMMMMMMMMMMMMMMMMMMMMMMMMMMMM"
    }
}

private class RunKey(var text: String, var fontKey: Int) {
    override fun hashCode(): Int = text.hashCode() * 31 + fontKey
    override fun equals(other: Any?): Boolean =
        other is RunKey && other.fontKey == fontKey && other.text == text
}
