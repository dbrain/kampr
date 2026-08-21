package dev.kampr.shared.ui

import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.systemBars
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// What the system draws over the app, as a value rather than a modifier.
//
// `enableEdgeToEdge()` is half a contract: the app is allowed to paint under the status bar and
// the gesture handle, and it then has to keep its *content* out of them. Nothing did, so the
// gesture handle landed on top of the "Pane" label on every portrait screen.
//
// A value rather than `Modifier.systemBarsPadding()` for two reasons. The terminal deliberately
// paints to the edges while its controls stay clear, which a blanket padding at the root would
// take away. And a composition local can be *provided* by a test, where the real insets are always
// zero — which is why every layout test in this suite was blind to this.
//
// Four sides, not two: in landscape the bars move. A three-button navigation bar goes to whichever
// end of the screen the rotation put it at, and a cutout takes the other — which is exactly where
// the key row's outermost caps and the pane header's back arrow live. `left` and `right` are
// physical, so they are applied with `absolutePadding` and never with `start`/`end`.
data class SafeArea(
    val top: Dp,
    val bottom: Dp,
    val left: Dp = 0.dp,
    val right: Dp = 0.dp,
) {
    companion object {
        val None = SafeArea(0.dp, 0.dp)
    }
}

val LocalSafeArea = staticCompositionLocalOf { SafeArea.None }

@Composable
fun systemSafeArea(): SafeArea {
    val padding = WindowInsets.systemBars.asPaddingValues()
    val direction = LocalLayoutDirection.current
    return SafeArea(
        top = padding.calculateTopPadding(),
        bottom = padding.calculateBottomPadding(),
        left = padding.calculateLeftPadding(direction),
        right = padding.calculateRightPadding(direction),
    )
}
