package dev.kampr.shared.theme

import androidx.compose.foundation.background
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

object Kampr {
    val tokens: KamprTokens
        @Composable get() = LocalTokens.current
}

@Composable
fun KamprTheme(spec: ThemeSpec, scale: TypeScale, content: @Composable () -> Unit) {
    val fonts = resolveFonts(spec)
    if (fonts == null) {
        Box(Modifier.fillMaxSize().background(spec.palette.bg))
        return
    }
    val type = remember(fonts, spec, scale) { typography(fonts, spec.label, scale) }
    val tokens = remember(fonts, spec, type) { KamprTokens(spec, fonts, type) }
    CompositionLocalProvider(LocalTokens provides tokens, content = content)
}
