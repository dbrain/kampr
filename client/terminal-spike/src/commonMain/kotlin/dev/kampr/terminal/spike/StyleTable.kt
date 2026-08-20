package dev.kampr.terminal.spike

const val FONT_BOLD = 1
const val FONT_ITALIC = 2
const val FONT_UNDERLINE = 4
const val FONT_STRIKE = 8

class StyleTable {
    private var cap = 64
    var fg = IntArray(cap)
        private set
    var bg = IntArray(cap)
        private set
    var fontKey = IntArray(cap)
        private set

    init {
        append(StylesMsg(0, listOf(Style())))
    }

    fun append(msg: StylesMsg) {
        val need = msg.from + msg.styles.size
        if (need > cap) {
            while (cap < need) cap *= 2
            fg = fg.copyOf(cap)
            bg = bg.copyOf(cap)
            fontKey = fontKey.copyOf(cap)
        }
        msg.styles.forEachIndexed { i, s -> resolve(msg.from + i, s) }
    }

    private fun resolve(id: Int, s: Style) {
        var f = when (val c = s.fg) {
            is ColorSpec.Default -> Palette.DEFAULT_FG
            is ColorSpec.Indexed -> Palette.xterm256[c.v and 0xFF]
            is ColorSpec.Rgb -> Palette.argb(c.r, c.g, c.b)
        }
        var b = when (val c = s.bg) {
            is ColorSpec.Default -> Palette.DEFAULT_BG
            is ColorSpec.Indexed -> Palette.xterm256[c.v and 0xFF]
            is ColorSpec.Rgb -> Palette.argb(c.r, c.g, c.b)
        }
        if (s.dim) f = Palette.dim(f)
        if (s.reverse) {
            val t = f; f = b; b = t
        }
        if (s.hidden) f = b
        fg[id] = f
        bg[id] = b
        fontKey[id] = (if (s.bold) FONT_BOLD else 0) or
            (if (s.italic) FONT_ITALIC else 0) or
            (if (s.underline) FONT_UNDERLINE else 0) or
            (if (s.strike) FONT_STRIKE else 0)
    }
}
