package dev.kampr.shared

import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Ground
import dev.kampr.shared.theme.KamprFonts
import dev.kampr.shared.theme.KamprTokens
import dev.kampr.shared.theme.LocalTokens
import dev.kampr.shared.theme.TypeScale
import dev.kampr.shared.theme.on
import dev.kampr.shared.theme.themeOf
import dev.kampr.shared.theme.typography
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.SafeArea

// What a pixel_6 reports: a status bar with a punch-hole in it, and a gesture handle. 1080×2400
// at 480 dpi, which is the profile every one of these defects has been found on.
val BARS = SafeArea(top = 44.dp, bottom = 46.dp)

// Rotated with three-button navigation, which is the posture that moves the bar to a side and
// takes the other with a cutout. Zero under gestures, which is why the emulator hid this.
val SIDE_BARS = listOf(
    SafeArea(top = 24.dp, bottom = 0.dp, left = 48.dp, right = 0.dp),
    SafeArea(top = 24.dp, bottom = 0.dp, left = 0.dp, right = 48.dp),
)

fun phoneTokens(): KamprTokens = themeOf("soft").on(Ground.Dark).let { spec ->
    val fonts = KamprFonts(FontFamily.Default, FontFamily.Monospace, FontFamily.Monospace)
    KamprTokens(spec, fonts, typography(fonts, spec.label, TypeScale.Phone))
}

// Every layout assertion in this suite measures Compose's own semantics tree, which knows nothing
// about what SystemUI paints on top of it — so a full suite passed while the gesture handle sat on
// the word "Pane". `LocalSafeArea` exists so a test can put the bars back.
@Composable
fun Bars(bars: SafeArea = BARS, content: @Composable () -> Unit) {
    CompositionLocalProvider(LocalTokens provides phoneTokens(), LocalSafeArea provides bars, content = content)
}
