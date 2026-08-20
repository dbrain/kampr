package dev.kampr.shared.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color
import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.Style

// ADR 0009: the terminal keeps a dark ground under every theme and both grounds, because
// truecolour and the 256-cube are absolute values Kampr must not touch and they are what the
// harnesses mostly emit. Only these 16 slots are ours to theme, so they are authored once per
// theme against that theme's own terminal ground — there is no light variant to author.
@Immutable
class TerminalSkin internal constructor(
    val ground: Color,
    val ink: Color,
    val selectionWash: Color,
    val linkInk: Color,
    internal val slots: IntArray,
)

private fun skin(
    ground: Long,
    ink: Long,
    wash: Long,
    link: Long,
    vararg slots: Long,
) = TerminalSkin(
    ground = Color(ground),
    ink = Color(ink),
    selectionWash = Color(wash),
    linkInk = Color(link),
    slots = IntArray(16) { slots[it].toInt() },
)

private val terminalSkins: Map<ThemeId, TerminalSkin> = mapOf(
    ThemeId.Soft to skin(
        0xFF0B0D12L, 0xFFDDE3EEL, 0x4D6EA8FEL, 0xFF9CC4FFL,
        0xFF1B1F27L, 0xFFF2707AL, 0xFF5FD68FL, 0xFFE9C46AL,
        0xFF7FB0FFL, 0xFFC79BE0L, 0xFF5FD2D2L, 0xFFC6CCD9L,
        0xFF7C8496L, 0xFFFF8E96L, 0xFF86E9ABL, 0xFFF7D98CL,
        0xFFA6C8FFL, 0xFFDDB8F0L, 0xFF8CE7E7L, 0xFFF2F5FAL,
    ),
    ThemeId.Phosphor to skin(
        0xFF050705L, 0xFFC9D5C7L, 0x47F5C542L, 0xFFFFD968L,
        0xFF131A13L, 0xFFFF5F4AL, 0xFF57C46BL, 0xFFF5C542L,
        0xFF58A6B8L, 0xFFD98BA0L, 0xFF7FE0C8L, 0xFFC4CFC2L,
        0xFF6F7D6DL, 0xFFFF8A72L, 0xFF7FDC8EL, 0xFFFFD968L,
        0xFF7FC8D8L, 0xFFEEADBEL, 0xFFA6F0DEL, 0xFFE9F0E7L,
    ),
    ThemeId.Warm to skin(
        0xFF15110DL, 0xFFEEE4D5L, 0x47E0A458L, 0xFFF0BD7CL,
        0xFF241F1AL, 0xFFD9604EL, 0xFF7FB069L, 0xFFE0A458L,
        0xFF7A93C4L, 0xFFC48AA8L, 0xFF79B8B0L, 0xFFD8CDBCL,
        0xFF8A7D6DL, 0xFFF07A64L, 0xFF97C77FL, 0xFFF0BD7CL,
        0xFF9AB0DAL, 0xFFDCA5C1L, 0xFF95D0C8L, 0xFFF6EFE3L,
    ),
    ThemeId.Brutalist to skin(
        0xFF000000L, 0xFFFFFFFFL, 0x59FF2B1FL, 0xFFFF6A60L,
        0xFF1A1A1AL, 0xFFFF2B1FL, 0xFF00E05AL, 0xFFFFE500L,
        0xFF4D7CFFL, 0xFFFF3DDBL, 0xFF00E5E5L, 0xFFE6E6E6L,
        0xFF8A8A8AL, 0xFFFF6A60L, 0xFF5CFF9EL, 0xFFFFF35CL,
        0xFF8FAEFFL, 0xFFFF8AE8L, 0xFF6BFFFFL, 0xFFFFFFFFL,
    ),
)

private val cubeSteps = intArrayOf(0, 95, 135, 175, 215, 255)

// Indices 16..255 are absolute values a program asked for by number, identical under every
// theme. Built once rather than per palette.
private val extended: IntArray = IntArray(240) { i ->
    if (i < 216) {
        val r = cubeSteps[i / 36]
        val g = cubeSteps[(i / 6) % 6]
        val b = cubeSteps[i % 6]
        0xFF000000.toInt() or (r shl 16) or (g shl 8) or b
    } else {
        val v = 8 + (i - 216) * 10
        0xFF000000.toInt() or (v shl 16) or (v shl 8) or v
    }
}

// SGR 2 composites over whatever the cell's background actually is. At the old 0.55 the faint
// form of bright-black landed at 1.63:1 on the ground, which is where agents put their least
// important text; 0.75 floors every slot at 3:1 (ADR 0009).
private const val FAINT = 0.75f

@Immutable
class TerminalPalette(private val skin: TerminalSkin) {
    val selectionWash: Color get() = skin.selectionWash
    val linkInk: Color get() = skin.linkInk

    fun slot(index: Int): Int =
        if (index < 16) skin.slots[index] else extended[index - 16]

    private fun resolve(spec: ColorSpec, fallback: Color): Color = when (spec) {
        ColorSpec.Default -> fallback
        is ColorSpec.Indexed -> Color(slot(spec.v.coerceIn(0, 255)))
        is ColorSpec.Rgb -> Color(spec.r.coerceIn(0, 255), spec.g.coerceIn(0, 255), spec.b.coerceIn(0, 255))
    }

    fun foreground(style: Style): Color {
        val base = if (style.reverse) resolve(style.bg, skin.ground) else resolve(style.fg, skin.ink)
        return when {
            style.hidden -> Color.Transparent
            style.dim -> base.copy(alpha = FAINT)
            else -> base
        }
    }

    fun background(style: Style): Color =
        if (style.reverse) resolve(style.fg, skin.ink) else resolve(style.bg, skin.ground)
}

fun terminalSkin(id: ThemeId): TerminalSkin = terminalSkins.getValue(id)

fun KamprTokens.terminalPalette(): TerminalPalette = TerminalPalette(terminalSkin(spec.id))
