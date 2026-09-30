package dev.kampr.shared.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

// Whether the app's own chrome sits under a screen at the bottom of the window — which is the one
// fact that decides who owes the gesture handle, and on a phone is also whether the tab bar is
// drawn at all.
//
// Only a phone's tabs, and never under a pane: every row of height there is the pane's, and
// `onBack` already leads out to them. The desk has nothing down there — the sidebar says what a
// status strip used to, and a held pane says so in its own header.
internal fun bottomChrome(breakpoint: Breakpoint, screen: Screen): Boolean =
    breakpoint != Breakpoint.Desktop && screen !is Screen.Pane

// Every screen but one. A pane hosts the terminal, which draws its grid on a canvas and carries
// its own selection — anchor, head, block mode, a copy of the logical line — off gestures a
// container above it would take first (TerminalView, SelectionLayer). The transcript a pane can
// also show wraps its own.
internal fun screenSelects(screen: Screen): Boolean = screen !is Screen.Pane

// Where a screen goes, and what it is told about the bottom of the window. The scaffold is the one
// thing that can see whether its own chrome is under the screen, so it is the thing that says so.
@Composable
internal fun ScreenBody(modifier: Modifier, chrome: Boolean, selects: Boolean, content: @Composable () -> Unit) {
    Box(modifier) {
        if (selects) SelectionContainer { BottomEdgeHeldBelow(chrome, content) }
        else BottomEdgeHeldBelow(chrome, content)
    }
}

// A phone, either way up: the screen, and under it the tab bar that leads out of it. One shape for
// both postures, because the difference between them is which screens they draw and not how the
// bottom of the window is put together — and because a test that arranges the pieces itself proves
// nothing about the app that arranges them differently.
@Composable
internal fun PhoneScaffold(
    breakpoint: Breakpoint,
    screen: Screen,
    onSelect: (Tab) -> Unit,
    body: @Composable () -> Unit,
) {
    val chrome = bottomChrome(breakpoint, screen)
    Column(Modifier.fillMaxSize()) {
        ScreenBody(Modifier.weight(1f).screenInset(screen), chrome, screenSelects(screen), body)
        if (chrome) BottomNav(tabFor(screen), onSelect)
    }
}
