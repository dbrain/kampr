package dev.kampr.terminal.spike

object Palette {
    const val DEFAULT_FG = 0xFFD4D4D8.toInt()
    const val DEFAULT_BG = 0xFF0E0E12.toInt()

    private val ansi16 = intArrayOf(
        0xFF1E1E24.toInt(), 0xFFE05252.toInt(), 0xFF5FBF6E.toInt(), 0xFFD7B75B.toInt(),
        0xFF5A9BE0.toInt(), 0xFFB07CD6.toInt(), 0xFF4FC1C9.toInt(), 0xFFC8C8CE.toInt(),
        0xFF4A4A55.toInt(), 0xFFFF7B72.toInt(), 0xFF7EE787.toInt(), 0xFFF2D06B.toInt(),
        0xFF79B8FF.toInt(), 0xFFD2A8FF.toInt(), 0xFF6FE3EC.toInt(), 0xFFFFFFFF.toInt(),
    )

    val xterm256: IntArray = IntArray(256).also { p ->
        for (i in 0 until 16) p[i] = ansi16[i]
        val steps = intArrayOf(0, 95, 135, 175, 215, 255)
        var i = 16
        for (r in 0 until 6) for (g in 0 until 6) for (b in 0 until 6) {
            p[i++] = argb(steps[r], steps[g], steps[b])
        }
        for (n in 0 until 24) {
            val v = 8 + n * 10
            p[i++] = argb(v, v, v)
        }
    }

    fun argb(r: Int, g: Int, b: Int): Int =
        (0xFF shl 24) or ((r and 0xFF) shl 16) or ((g and 0xFF) shl 8) or (b and 0xFF)

    fun dim(c: Int): Int {
        val r = ((c shr 16 and 0xFF) * 6) / 10
        val g = ((c shr 8 and 0xFF) * 6) / 10
        val b = ((c and 0xFF) * 6) / 10
        return argb(r, g, b)
    }
}
