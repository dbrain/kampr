package dev.kampr.shared.ui

import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.systemBars
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Modifier
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
//
// `ime` is the fifth side, and the one the system moves: the on-screen keyboard covers the bottom
// of the window without resizing it, so a surface that ends at the window's bottom edge ends up
// underneath it. The conversation composer did, and typing into a reply box you cannot see is what
// a reader gets when nothing pays for this. It is measured from the bottom of the window, which is
// why exactly one surface — the one that reaches that edge — applies it, and nothing inside has to
// reconcile it against its own position.
data class SafeArea(
    val top: Dp,
    val bottom: Dp,
    val left: Dp = 0.dp,
    val right: Dp = 0.dp,
    val ime: Dp = 0.dp,
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
    return safeAreaOf(
        top = padding.calculateTopPadding(),
        bottom = padding.calculateBottomPadding(),
        left = padding.calculateLeftPadding(direction),
        right = padding.calculateRightPadding(direction),
        ime = imeInset(),
    )
}

// Not `WindowInsets.ime`: that is Android's answer and the web's is zero, because a browser shrinks
// the visual viewport and leaves the canvas the size it always was.
@Composable
internal expect fun imeInset(): Dp

// The keyboard is drawn over the navigation bar, and the system keeps reporting the bar's height
// the whole time. Standing clear of both is a strip of dead ground between the last control and
// the keys — one bottom inset tall, which is exactly the gap the operator saw under the key row.
internal fun safeAreaOf(top: Dp, bottom: Dp, left: Dp, right: Dp, ime: Dp): SafeArea =
    SafeArea(top, if (ime > 0.dp) 0.dp else bottom, left, right, ime)

// Where the keyboard is paid for: a surface that reaches the bottom of the window, so the inset
// applies whole and nothing below it has to be subtracted. Everything inside is then laid out
// against a window that already stops at the keys, and needs to know nothing about them.
@Composable
fun Modifier.keyboardInset(): Modifier = padding(bottom = LocalSafeArea.current.ime)

// The gesture handle is owed by whatever ends at the bottom of the window, and by nothing else. A
// container that puts its own chrome down there — the bottom navigation, the desktop status strip
// — takes the bottom edge off everything it holds, so the bar above it does not pay a second time
// and leave a dead strip between the two.
//
// The container decides because only the container can see what is under it. A child that worked
// it out for itself, by reconciling its own `positionInWindow` against `containerSize`, was left
// holding exactly the display cutout: nothing on an emulator that reports none, 128 px on every
// phone with a punch-hole, and 581 green tests either way.
@Composable
fun BottomEdgeHeldBelow(held: Boolean, content: @Composable () -> Unit) {
    val safe = LocalSafeArea.current
    CompositionLocalProvider(
        LocalSafeArea provides if (held) safe.copy(bottom = 0.dp) else safe,
        content = content,
    )
}
