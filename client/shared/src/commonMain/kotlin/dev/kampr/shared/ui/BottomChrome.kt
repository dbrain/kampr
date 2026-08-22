package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

// Whether the app's own chrome sits under a screen at the bottom of the window — which is the one
// fact that decides who owes the gesture handle, and on a phone is also whether the tab bar is
// drawn at all.
//
// The desktop always ends in its status strip. Landscape keeps the tabs off a pane, where every
// row of height is the terminal's and `onBack` already leads out. Portrait always has them: the
// keyboard takes the bar's *height* rather than its presence, so this answer no longer moves
// under the keys — and neither does what the pane above is told it owes.
internal fun bottomChrome(breakpoint: Breakpoint, screen: Screen): Boolean = when (breakpoint) {
    Breakpoint.Desktop, Breakpoint.Portrait -> true
    Breakpoint.Landscape -> screen !is Screen.Pane
}

// How much of the tab bar the keys are drawn over. A pane is the one screen whose bar they take —
// the key row and the reply box are meant to sit on the keys, and a tab bar between them is a
// strip of chrome nobody is reaching for mid-sentence.
//
// `edge` is the window's own furniture, read above the keyboard. The subtraction is what the bar
// has already stopped paying for: it stands off the gesture handle, and the keys are over the
// handle before they are over anything of the bar's own.
internal fun barCovered(breakpoint: Breakpoint, screen: Screen, edge: SafeArea): Dp =
    if (breakpoint == Breakpoint.Portrait && screen is Screen.Pane) {
        (edge.ime - edge.bottom).coerceAtLeast(0.dp)
    } else {
        0.dp
    }

// Chrome the keyboard is drawn over, revealed as the keys retreat rather than switched on once
// they have gone. A boolean keyed on `ime == 0.dp` has no partially-uncovered state, so the bar
// arrived whole, at the bottom, in the single frame after a 250 ms animation ended — which is
// what "jumps into vision" is.
//
// Laid out from the top and clipped at the bottom, so what shows is always the bar in the place
// it will finally rest: the keys uncover it, and it never moves.
@Composable
private fun Uncovered(covered: Dp, content: @Composable () -> Unit) {
    Layout(content, Modifier.clipToBounds()) { measurables, constraints ->
        val bar = measurables.first()
            .measure(constraints.copy(minHeight = 0, maxHeight = Constraints.Infinity))
        val height = (bar.height - covered.roundToPx()).coerceIn(0, bar.height)
        layout(bar.width, height) { bar.place(0, 0) }
    }
}

// Where a screen goes, and what it is told about the bottom of the window. The scaffold is the one
// thing that can see whether its own chrome is under the screen, so it is the thing that says so.
@Composable
internal fun ScreenBody(modifier: Modifier, chrome: Boolean, content: @Composable () -> Unit) {
    Box(modifier) { BottomEdgeHeldBelow(chrome, content) }
}

// A phone, either way up: the screen, and under it the tab bar that leads out of it. One shape for
// both postures, because the difference between them is which screens they draw and not how the
// bottom of the window is put together — and because a test that arranges the pieces itself proves
// nothing about the app that arranges them differently.
//
// `edge` is the window's own furniture, read above the keyboard rather than inside it.
@Composable
internal fun PhoneScaffold(
    breakpoint: Breakpoint,
    screen: Screen,
    edge: SafeArea,
    onSelect: (Tab) -> Unit,
    body: @Composable () -> Unit,
) {
    val chrome = bottomChrome(breakpoint, screen)
    Column(Modifier.fillMaxSize()) {
        ScreenBody(Modifier.weight(1f).screenInset(screen), chrome, body)
        if (chrome) {
            Uncovered(barCovered(breakpoint, screen, edge)) { BottomNav(tabFor(screen), onSelect) }
        }
    }
}
