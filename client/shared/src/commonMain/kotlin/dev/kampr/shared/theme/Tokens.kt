package dev.kampr.shared.theme

import androidx.compose.runtime.Immutable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.TextUnit
import androidx.compose.ui.unit.dp

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
enum class Ground { Dark, Light }

@Immutable
enum class ThemeMode(val key: String, val title: String) {
    System("system", "System"),
    Dark("dark", "Dark"),
    Light("light", "Light"),
}

fun modeOf(key: String?): ThemeMode = ThemeMode.entries.firstOrNull { it.key == key } ?: ThemeMode.System

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
    val ground: Ground,
    val palette: Palette,
    val radii: Radii,
    val card: BorderSpec,
    val chrome: BorderSpec,
    val label: LabelSpec,
    val ui: FamilyId,
    val mono: FamilyId,
)

@Immutable
data class ThemeFamily(val dark: ThemeSpec, val light: ThemeSpec) {
    val id: ThemeId get() = dark.id
    fun on(ground: Ground): ThemeSpec = if (ground == Ground.Light) light else dark
}
