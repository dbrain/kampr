package dev.kampr.shared.theme

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em

private val defaultLabel = LabelSpec(uppercase = false, tracking = 0.em, weight = FontWeight.W700)

private fun family(
    id: ThemeId,
    dark: Palette,
    light: Palette,
    radii: Radii,
    label: LabelSpec,
    ui: FamilyId,
    mono: FamilyId,
    cardWidth: Dp = 1.dp,
    cardOnLine: Boolean = false,
    chromeWidth: Dp = 1.dp,
): ThemeFamily {
    fun spec(ground: Ground, p: Palette) = ThemeSpec(
        id = id,
        ground = ground,
        palette = p,
        radii = radii,
        card = BorderSpec(cardWidth, if (cardOnLine) p.line else Color.Transparent),
        chrome = BorderSpec(chromeWidth, p.line),
        label = label,
        ui = ui,
        mono = mono,
    )
    return ThemeFamily(spec(Ground.Dark, dark), spec(Ground.Light, light))
}

val SoftFamily = family(
    id = ThemeId.Soft,
    dark = Palette(
        bg = Color(0xFF0E0F13), bar = Color(0xFF12141A), surface = Color(0xFF171A21),
        surface2 = Color(0xFF090A0D), raise = Color(0xFF22262F), line = Color(0xFF1E2129),
        text = Color(0xFFE8EAF0), dim = Color(0xFF8D93A3), mute = Color(0xFF545A68),
        accent = Color(0xFF6EA8FE), accentHi = Color(0xFF9CC4FF), onAccent = Color(0xFF0E0F13),
        accentSoft = Color(0x50162030),
        blocked = Color(0xFFFF6B6B), blockedBg = Color(0xFF2C1B20), working = Color(0xFFFFC857),
        idle = Color(0xFF5B6172), done = Color(0xFF58D68D),
    ),
    light = Palette(
        bg = Color(0xFFF7F8FB), bar = Color(0xFFEEF1F7), surface = Color(0xFFFFFFFF),
        surface2 = Color(0xFFE7EBF2), raise = Color(0xFFE2E7F0), line = Color(0xFFD8DEE9),
        text = Color(0xFF12151B), dim = Color(0xFF565D6D), mute = Color(0xFF6A7183),
        accent = Color(0xFF2560CC), accentHi = Color(0xFF17439A), onAccent = Color(0xFFFFFFFF),
        accentSoft = Color(0x30A8C4F0),
        blocked = Color(0xFFB8281F), blockedBg = Color(0xFFFBE4E1), working = Color(0xFF8A5A00),
        idle = Color(0xFF9AA1B2), done = Color(0xFF0F6B45),
    ),
    radii = Radii(lg = 18.dp, md = 13.dp, sm = 10.dp),
    label = defaultLabel,
    ui = FamilyId.Manrope,
    mono = FamilyId.IbmPlexMono,
)

val PhosphorFamily = family(
    id = ThemeId.Phosphor,
    dark = Palette(
        bg = Color(0xFF0A0B0A), bar = Color(0xFF0C0E0C), surface = Color(0xFF101210),
        surface2 = Color(0xFF070807), raise = Color(0xFF171A17), line = Color(0xFF1D211D),
        text = Color(0xFFCDD6CD), dim = Color(0xFF6B756B), mute = Color(0xFF4D554D),
        accent = Color(0xFFF5C542), accentHi = Color(0xFFFFD968), onAccent = Color(0xFF0A0B0A),
        accentSoft = Color(0xFF16130A),
        blocked = Color(0xFFFF5F56), blockedBg = Color(0xFF150D0C), working = Color(0xFFF5C542),
        idle = Color(0xFF4A534A), done = Color(0xFF57C46B),
    ),
    light = Palette(
        bg = Color(0xFFF3F5EF), bar = Color(0xFFEAEEE4), surface = Color(0xFFFBFCF8),
        surface2 = Color(0xFFE2E7D9), raise = Color(0xFFDDE3D3), line = Color(0xFFCDD5C1),
        text = Color(0xFF0F150D), dim = Color(0xFF4D5749), mute = Color(0xFF68725F),
        accent = Color(0xFF7A5400), accentHi = Color(0xFF5C3F00), onAccent = Color(0xFFFFFFFF),
        accentSoft = Color(0xFFF0E6C8),
        blocked = Color(0xFFA82015), blockedBg = Color(0xFFF8E3E0), working = Color(0xFF7A5400),
        idle = Color(0xFF98A291), done = Color(0xFF1D5E2A),
    ),
    radii = Radii(lg = 2.dp, md = 2.dp, sm = 2.dp),
    label = LabelSpec(uppercase = true, tracking = 0.16.em, weight = FontWeight.W700),
    ui = FamilyId.JetBrainsMono,
    mono = FamilyId.JetBrainsMono,
    cardOnLine = true,
)

val WarmFamily = family(
    id = ThemeId.Warm,
    dark = Palette(
        bg = Color(0xFF12100E), bar = Color(0xFF171412), surface = Color(0xFF1A1714),
        surface2 = Color(0xFF0D0B0A), raise = Color(0xFF241F1B), line = Color(0xFF221E1A),
        text = Color(0xFFEFE7DC), dim = Color(0xFF8D8175), mute = Color(0xFF5F574E),
        accent = Color(0xFFE0A458), accentHi = Color(0xFFF0BD7C), onAccent = Color(0xFF12100E),
        accentSoft = Color(0xFF211A12),
        blocked = Color(0xFFE0685A), blockedBg = Color(0xFF2A1613), working = Color(0xFFE0A458),
        idle = Color(0xFF4A423A), done = Color(0xFF7FB069),
    ),
    light = Palette(
        bg = Color(0xFFFAF6EF), bar = Color(0xFFF3ECDF), surface = Color(0xFFFFFDF8),
        surface2 = Color(0xFFEFE7D8), raise = Color(0xFFEAE0CD), line = Color(0xFFDED3BF),
        text = Color(0xFF1C1610), dim = Color(0xFF635749), mute = Color(0xFF7B6F5D),
        accent = Color(0xFF8F5210), accentHi = Color(0xFF6E3C06), onAccent = Color(0xFFFFFDF8),
        accentSoft = Color(0xFFF2E3CB),
        blocked = Color(0xFFA83521), blockedBg = Color(0xFFF8E2DA), working = Color(0xFF8F5210),
        idle = Color(0xFFAA9C86), done = Color(0xFF3A6127),
    ),
    radii = Radii(lg = 14.dp, md = 12.dp, sm = 9.dp),
    label = LabelSpec(uppercase = true, tracking = 0.13.em, weight = FontWeight.W600),
    ui = FamilyId.InstrumentSans,
    mono = FamilyId.JetBrainsMono,
)

val BrutalistFamily = family(
    id = ThemeId.Brutalist,
    dark = Palette(
        bg = Color(0xFF000000), bar = Color(0xFF000000), surface = Color(0xFF000000),
        surface2 = Color(0xFF000000), raise = Color(0xFF000000), line = Color(0xFFFFFFFF),
        text = Color(0xFFFFFFFF), dim = Color(0xFF8A8A8A), mute = Color(0xFF5C5C5C),
        accent = Color(0xFFFF2B1F), accentHi = Color(0xFFFF6A60), onAccent = Color(0xFF000000),
        accentSoft = Color(0xFF000000),
        blocked = Color(0xFFFF2B1F), blockedBg = Color(0xFF000000), working = Color(0xFFFFFFFF),
        idle = Color(0xFF333333), done = Color(0xFFFFFFFF),
    ),
    light = Palette(
        bg = Color(0xFFFFFFFF), bar = Color(0xFFFFFFFF), surface = Color(0xFFFFFFFF),
        surface2 = Color(0xFFFFFFFF), raise = Color(0xFFFFFFFF), line = Color(0xFF000000),
        text = Color(0xFF000000), dim = Color(0xFF4A4A4A), mute = Color(0xFF5E5E5E),
        accent = Color(0xFFCC0A00), accentHi = Color(0xFF960700), onAccent = Color(0xFFFFFFFF),
        accentSoft = Color(0xFFFFFFFF),
        blocked = Color(0xFFCC0A00), blockedBg = Color(0xFFFFFFFF), working = Color(0xFF000000),
        idle = Color(0xFFB5B5B5), done = Color(0xFF000000),
    ),
    radii = Radii(lg = 0.dp, md = 0.dp, sm = 0.dp),
    label = LabelSpec(uppercase = true, tracking = 0.2.em, weight = FontWeight.W700),
    ui = FamilyId.Archivo,
    mono = FamilyId.IbmPlexMono,
    cardWidth = 2.dp,
    cardOnLine = true,
    chromeWidth = 2.dp,
)

val AllFamilies = listOf(SoftFamily, PhosphorFamily, WarmFamily, BrutalistFamily)

val AllThemes: List<ThemeSpec> = AllFamilies.map { it.dark }

val SoftTheme = SoftFamily.dark
val PhosphorTheme = PhosphorFamily.dark
val WarmTheme = WarmFamily.dark
val BrutalistTheme = BrutalistFamily.dark

fun ThemeSpec.on(ground: Ground): ThemeSpec =
    if (ground == this.ground) this else AllFamilies.first { it.id == id }.on(ground)

fun themeOf(key: String?): ThemeSpec = (AllFamilies.firstOrNull { it.id.key == key } ?: SoftFamily).dark
