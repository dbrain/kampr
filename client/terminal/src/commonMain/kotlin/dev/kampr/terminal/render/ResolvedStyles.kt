package dev.kampr.terminal.render

import androidx.compose.ui.graphics.toArgb
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.theme.TerminalPalette

const val FONT_BOLD = 1
const val FONT_ITALIC = 2
const val FONT_UNDERLINE = 4
const val FONT_STRIKE = 8

// Style ids resolve to packed ARGB once per table move rather than per cell per frame.
class ResolvedStyles(private val palette: TerminalPalette) {
    var fg = IntArray(1)
        private set
    var bg = IntArray(1)
        private set
    var fontKey = IntArray(1)
        private set

    private var seen = -1

    val defaultBg: Int get() = bg[0]

    fun sync(table: StyleTable) {
        if (table.version == seen) return
        seen = table.version
        val size = table.size
        fg = IntArray(size)
        bg = IntArray(size)
        fontKey = IntArray(size)
        for (id in 0 until size) {
            val style = table[id]
            fg[id] = palette.foreground(style).toArgb()
            bg[id] = palette.background(style).toArgb()
            fontKey[id] = (if (style.bold) FONT_BOLD else 0) or
                (if (style.italic) FONT_ITALIC else 0) or
                (if (style.underline) FONT_UNDERLINE else 0) or
                (if (style.strike) FONT_STRIKE else 0)
        }
    }

    fun clamp(id: Int): Int = if (id in fg.indices) id else 0
}
