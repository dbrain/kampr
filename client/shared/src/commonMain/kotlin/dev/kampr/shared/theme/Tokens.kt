package dev.kampr.shared.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em

@Immutable
data class Palette(
    val bg: Color,
    val bar: Color,
    val surface: Color,
    val surface2: Color,
    val raise: Color,
    val line: Color,
    val text: Color,
    val dim: Color,
    val mute: Color,
    val accent: Color,
    val accentHi: Color,
    val onAccent: Color,
    val accentSoft: Color,
    val blocked: Color,
    val blockedBg: Color,
    val working: Color,
    val idle: Color,
    val done: Color,
)

@Immutable
data class Radii(val lg: Dp, val md: Dp, val sm: Dp) {
    val pill: Dp = 999.dp
}

@Immutable
data class BorderSpec(val width: Dp, val color: Color) {
    val visible: Boolean get() = width > 0.dp && color.alpha > 0f
}

@Immutable
data class LabelSpec(val uppercase: Boolean, val tracking: TextUnit, val weight: FontWeight)

@Immutable
enum class FamilyId { Manrope, IbmPlexMono, JetBrainsMono, InstrumentSans, Archivo }

@Immutable
enum class ThemeId(val key: String, val title: String, val credit: String) {
    Soft("soft", "Soft native", "Manrope · IBM Plex Mono · 18px radii"),
    Phosphor("phosphor", "Phosphor", "JetBrains Mono throughout · 2px radii"),
    Warm("warm", "Warm editorial", "Instrument Sans · JetBrains Mono · 14px radii"),
    Brutalist("brutalist", "Brutalist", "Archivo · IBM Plex Mono · zero radii"),
}

@Immutable
data class ThemeSpec(
    val id: ThemeId,
    val palette: Palette,
    val radii: Radii,
    val card: BorderSpec,
    val chrome: BorderSpec,
    val label: LabelSpec,
    val ui: FamilyId,
    val mono: FamilyId,
)

private val defaultLabel = LabelSpec(uppercase = false, tracking = 0.em, weight = FontWeight.W700)

val SoftTheme = ThemeSpec(
    id = ThemeId.Soft,
    palette = Palette(
        bg = Color(0xFF0E0F13), bar = Color(0xFF12141A), surface = Color(0xFF171A21),
        surface2 = Color(0xFF090A0D), raise = Color(0xFF22262F), line = Color(0xFF1E2129),
        text = Color(0xFFE8EAF0), dim = Color(0xFF8D93A3), mute = Color(0xFF545A68),
        accent = Color(0xFF6EA8FE), accentHi = Color(0xFF9CC4FF), onAccent = Color(0xFF0E0F13),
        accentSoft = Color(0x50162030),
        blocked = Color(0xFFFF6B6B), blockedBg = Color(0xFF2C1B20), working = Color(0xFFFFC857),
        idle = Color(0xFF5B6172), done = Color(0xFF58D68D),
    ),
    radii = Radii(lg = 18.dp, md = 13.dp, sm = 10.dp),
    card = BorderSpec(1.dp, Color.Transparent),
    chrome = BorderSpec(1.dp, Color(0xFF1E2129)),
    label = defaultLabel,
    ui = FamilyId.Manrope,
    mono = FamilyId.IbmPlexMono,
)

val PhosphorTheme = ThemeSpec(
    id = ThemeId.Phosphor,
    palette = Palette(
        bg = Color(0xFF0A0B0A), bar = Color(0xFF0C0E0C), surface = Color(0xFF101210),
        surface2 = Color(0xFF070807), raise = Color(0xFF171A17), line = Color(0xFF1D211D),
        text = Color(0xFFCDD6CD), dim = Color(0xFF6B756B), mute = Color(0xFF4D554D),
        accent = Color(0xFFF5C542), accentHi = Color(0xFFFFD968), onAccent = Color(0xFF0A0B0A),
        accentSoft = Color(0xFF16130A),
        blocked = Color(0xFFFF5F56), blockedBg = Color(0xFF150D0C), working = Color(0xFFF5C542),
        idle = Color(0xFF4A534A), done = Color(0xFF57C46B),
    ),
    radii = Radii(lg = 2.dp, md = 2.dp, sm = 2.dp),
    card = BorderSpec(1.dp, Color(0xFF1D211D)),
    chrome = BorderSpec(1.dp, Color(0xFF1D211D)),
    label = LabelSpec(uppercase = true, tracking = 0.16.em, weight = FontWeight.W700),
    ui = FamilyId.JetBrainsMono,
    mono = FamilyId.JetBrainsMono,
)

val WarmTheme = ThemeSpec(
    id = ThemeId.Warm,
    palette = Palette(
        bg = Color(0xFF12100E), bar = Color(0xFF171412), surface = Color(0xFF1A1714),
        surface2 = Color(0xFF0D0B0A), raise = Color(0xFF241F1B), line = Color(0xFF221E1A),
        text = Color(0xFFEFE7DC), dim = Color(0xFF8D8175), mute = Color(0xFF5F574E),
        accent = Color(0xFFE0A458), accentHi = Color(0xFFF0BD7C), onAccent = Color(0xFF12100E),
        accentSoft = Color(0xFF211A12),
        blocked = Color(0xFFE0685A), blockedBg = Color(0xFF2A1613), working = Color(0xFFE0A458),
        idle = Color(0xFF4A423A), done = Color(0xFF7FB069),
    ),
    radii = Radii(lg = 14.dp, md = 12.dp, sm = 9.dp),
    card = BorderSpec(1.dp, Color.Transparent),
    chrome = BorderSpec(1.dp, Color(0xFF221E1A)),
    label = LabelSpec(uppercase = true, tracking = 0.13.em, weight = FontWeight.W600),
    ui = FamilyId.InstrumentSans,
    mono = FamilyId.JetBrainsMono,
)

val BrutalistTheme = ThemeSpec(
    id = ThemeId.Brutalist,
    palette = Palette(
        bg = Color(0xFF000000), bar = Color(0xFF000000), surface = Color(0xFF000000),
        surface2 = Color(0xFF000000), raise = Color(0xFF000000), line = Color(0xFFFFFFFF),
        text = Color(0xFFFFFFFF), dim = Color(0xFF8A8A8A), mute = Color(0xFF5C5C5C),
        accent = Color(0xFFFF2B1F), accentHi = Color(0xFFFF6A60), onAccent = Color(0xFF000000),
        accentSoft = Color(0xFF000000),
        blocked = Color(0xFFFF2B1F), blockedBg = Color(0xFF000000), working = Color(0xFFFFFFFF),
        idle = Color(0xFF333333), done = Color(0xFFFFFFFF),
    ),
    radii = Radii(lg = 0.dp, md = 0.dp, sm = 0.dp),
    card = BorderSpec(2.dp, Color(0xFFFFFFFF)),
    chrome = BorderSpec(2.dp, Color(0xFFFFFFFF)),
    label = LabelSpec(uppercase = true, tracking = 0.2.em, weight = FontWeight.W700),
    ui = FamilyId.Archivo,
    mono = FamilyId.IbmPlexMono,
)

val AllThemes = listOf(SoftTheme, PhosphorTheme, WarmTheme, BrutalistTheme)

fun themeOf(key: String?): ThemeSpec = AllThemes.firstOrNull { it.id.key == key } ?: SoftTheme
