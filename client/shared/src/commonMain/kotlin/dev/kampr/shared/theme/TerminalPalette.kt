package dev.kampr.shared.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color
import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.Style

private val ansi16 = intArrayOf(
    0xFF1E2129.toInt(), 0xFFCC5555.toInt(), 0xFF58D68D.toInt(), 0xFFFFC857.toInt(),
    0xFF6EA8FE.toInt(), 0xFFB48EAD.toInt(), 0xFF6FD8D8.toInt(), 0xFFD8DCE4.toInt(),
    0xFF545A68.toInt(), 0xFFFF6B6B.toInt(), 0xFF7BE7A6.toInt(), 0xFFFFD98A.toInt(),
    0xFF9CC4FF.toInt(), 0xFFD3A4CE.toInt(), 0xFF97ECEC.toInt(), 0xFFFFFFFF.toInt(),
)

private val cubeSteps = intArrayOf(0, 95, 135, 175, 215, 255)

@Immutable
class TerminalPalette(private val defaultFg: Color, private val defaultBg: Color) {
    private val indexed: IntArray = IntArray(256).also { table ->
        for (i in 0 until 16) table[i] = ansi16[i]
        for (i in 16 until 232) {
            val n = i - 16
            val r = cubeSteps[n / 36]
            val g = cubeSteps[(n / 6) % 6]
            val b = cubeSteps[n % 6]
            table[i] = 0xFF000000.toInt() or (r shl 16) or (g shl 8) or b
        }
        for (i in 232 until 256) {
            val v = 8 + (i - 232) * 10
            table[i] = 0xFF000000.toInt() or (v shl 16) or (v shl 8) or v
        }
    }

    private fun resolve(spec: ColorSpec, fallback: Color): Color = when (spec) {
        ColorSpec.Default -> fallback
        is ColorSpec.Indexed -> Color(indexed[spec.v.coerceIn(0, 255)])
        is ColorSpec.Rgb -> Color(spec.r.coerceIn(0, 255), spec.g.coerceIn(0, 255), spec.b.coerceIn(0, 255))
    }

    fun foreground(style: Style): Color {
        val base = if (style.reverse) resolve(style.bg, defaultBg) else resolve(style.fg, defaultFg)
        return when {
            style.hidden -> Color.Transparent
            style.dim -> base.copy(alpha = 0.55f)
            else -> base
        }
    }

    fun background(style: Style): Color =
        if (style.reverse) resolve(style.fg, defaultFg) else resolve(style.bg, defaultBg)
}

fun KamprTokens.terminalPalette(): TerminalPalette = TerminalPalette(color.text, color.surface2)
