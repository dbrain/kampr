package dev.kampr.shared.theme

import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.Immutable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier

@Immutable
class KamprTokens(
    val spec: ThemeSpec,
    val fonts: KamprFonts,
    val type: KamprType,
) {
    val color: Palette get() = spec.palette
    val radii: Radii get() = spec.radii
    val card: BorderSpec get() = spec.card
    val chrome: BorderSpec get() = spec.chrome
    val label: LabelSpec get() = spec.label
}

val LocalTokens: ProvidableCompositionLocal<KamprTokens> = staticCompositionLocalOf {
    error("KamprTheme has not been applied")
}

// Null means "nobody upstream has decided", which is how an un-wired KamprTheme still follows
// prefers-color-scheme instead of silently pinning dark. Nested KamprTheme calls — the theme
// preview cards — inherit it rather than re-asking the system.
val LocalGround: ProvidableCompositionLocal<Ground?> = staticCompositionLocalOf { null }

object Kampr {
    val tokens: KamprTokens
        @Composable get() = LocalTokens.current
}

@Composable
fun groundOf(mode: ThemeMode): Ground = when (mode) {
    ThemeMode.Dark -> Ground.Dark
    ThemeMode.Light -> Ground.Light
    ThemeMode.System -> if (isSystemInDarkTheme()) Ground.Dark else Ground.Light
}

@Composable
fun KamprTheme(
    spec: ThemeSpec,
    scale: TypeScale,
    ground: Ground? = null,
    content: @Composable () -> Unit,
) {
    val resolved = ground ?: LocalGround.current ?: groundOf(ThemeMode.System)
    val grounded = remember(spec, resolved) { spec.on(resolved) }
    val fonts = resolveFonts(grounded)
    if (fonts == null) {
        Box(Modifier.fillMaxSize().background(grounded.palette.bg))
        return
    }
    val type = remember(fonts, grounded, scale) { typography(fonts, grounded.label, scale) }
    val tokens = remember(fonts, grounded, type) { KamprTokens(grounded, fonts, type) }
    CompositionLocalProvider(
        LocalTokens provides tokens,
        LocalGround provides resolved,
        content = content,
    )
}
