package dev.kampr.shared

import androidx.compose.ui.graphics.Color
import dev.kampr.shared.theme.AllFamilies
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.Palette
import dev.kampr.shared.theme.TerminalPalette
import dev.kampr.shared.theme.terminalSkin
import dev.kampr.shared.wire.ColorSpec
import dev.kampr.shared.wire.Style
import kotlin.math.max
import kotlin.math.min
import kotlin.math.pow
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val AA_BODY = 4.5f
private const val AA_LARGE = 3.0f

private fun linear(c: Float): Float =
    if (c <= 0.04045f) c / 12.92f else ((c + 0.055f) / 1.055f).pow(2.4f)

private fun luminance(c: Color): Float =
    0.2126f * linear(c.red) + 0.7152f * linear(c.green) + 0.0722f * linear(c.blue)

private fun contrast(a: Color, b: Color): Float {
    val la = luminance(a)
    val lb = luminance(b)
    return (max(la, lb) + 0.05f) / (min(la, lb) + 0.05f)
}

private fun over(fg: Color, bg: Color): Color = Color(
    red = fg.red * fg.alpha + bg.red * (1f - fg.alpha),
    green = fg.green * fg.alpha + bg.green * (1f - fg.alpha),
    blue = fg.blue * fg.alpha + bg.blue * (1f - fg.alpha),
)

private val pairs: List<Pair<String, (Palette) -> Pair<Color, Color>>> = listOf(
    "text/bg" to { p -> p.text to p.bg },
    "text/bar" to { p -> p.text to p.bar },
    "text/surface" to { p -> p.text to p.surface },
    "text/surface2" to { p -> p.text to p.surface2 },
    "text/raise" to { p -> p.text to p.raise },
    "dim/bg" to { p -> p.dim to p.bg },
    "dim/bar" to { p -> p.dim to p.bar },
    "dim/surface" to { p -> p.dim to p.surface },
    "mute/bg" to { p -> p.mute to p.bg },
    "mute/surface" to { p -> p.mute to p.surface },
    "accent/bg" to { p -> p.accent to p.bg },
    "accent/surface" to { p -> p.accent to p.surface },
    "accentHi/bg" to { p -> p.accentHi to p.bg },
    "onAccent/accent" to { p -> p.onAccent to p.accent },
    "blocked/bg" to { p -> p.blocked to p.bg },
    "blocked/blockedBg" to { p -> p.blocked to p.blockedBg },
    "working/bg" to { p -> p.working to p.bg },
    "done/bg" to { p -> p.done to p.bg },
)

class ThemeContrastTest {
    @Test
    fun lightGroundMeetsAaOnEveryTextPair() {
        val failures = mutableListOf<String>()
        for (family in AllFamilies) {
            val p = family.light.palette
            for ((name, pick) in pairs) {
                val (fg, bg) = pick(p)
                val r = contrast(fg, bg)
                if (r < AA_BODY) failures += "${family.id.key} $name = $r"
            }
        }
        assertTrue(failures.isEmpty(), "light ground below AA:\n" + failures.joinToString("\n"))
    }

    // ADR 0009: the 16 indexed slots are the only colours Kampr may redirect, so they are the
    // only ones it can be held to. Slot 0 is the ground by convention and is exempt.
    @Test
    fun everyAnsiSlotMeetsAaOnItsOwnTerminalGround() {
        val failures = mutableListOf<String>()
        for (family in AllFamilies) {
            val skin = terminalSkin(family.id)
            assertTrue(contrast(skin.ink, skin.ground) >= 7f, "${family.id.key} default ink")
            for (i in 1 until 16) {
                val slot = Color(skin.slots[i])
                val r = contrast(slot, skin.ground)
                if (r < AA_BODY) failures += "${family.id.key} slot $i = $r"
                val faint = contrast(over(slot.copy(alpha = 0.75f), skin.ground), skin.ground)
                if (faint < AA_LARGE) failures += "${family.id.key} slot $i faint = $faint"
            }
        }
        assertTrue(failures.isEmpty(), "ansi slots below AA:\n" + failures.joinToString("\n"))
    }

    @Test
    fun eachThemeHasItsOwnSixteenSlots() {
        val tables = AllFamilies.map { terminalSkin(it.id).slots.toList() }
        for (i in tables.indices) {
            for (j in i + 1 until tables.size) {
                assertTrue(tables[i] != tables[j], "themes $i and $j share one ANSI table")
            }
        }
    }

    @Test
    fun truecolourAndTheCubePassThroughUntouched() {
        for (family in AllFamilies) {
            val palette = TerminalPalette(terminalSkin(family.id))
            // The lightness agents author for a dark ground; it must survive verbatim.
            val authored = ColorSpec.Rgb(0xF6, 0xE2, 0xB7)
            assertEquals(
                Color(0xFFF6E2B7),
                palette.foreground(Style(fg = authored)),
                "${family.id.key} truecolour",
            )
            assertEquals(0xFF5FAF87.toInt(), palette.slot(72), "${family.id.key} 6x6x6 cube")
            assertEquals(0xFF8A8A8A.toInt(), palette.slot(245), "${family.id.key} greyscale ramp")
        }
    }

    @Test
    fun bothGroundsResolveFromOneFamily() {
        for (family in AllFamilies) {
            assertEquals(Ground.Dark, family.on(Ground.Dark).ground)
            assertEquals(Ground.Light, family.on(Ground.Light).ground)
            assertEquals(family.id, family.light.id)
            assertEquals(family.dark.radii, family.light.radii)
            assertEquals(family.dark.ui, family.light.ui)
        }
    }
}
