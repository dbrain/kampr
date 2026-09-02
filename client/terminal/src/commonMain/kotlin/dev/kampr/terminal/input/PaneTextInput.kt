package dev.kampr.terminal.input

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import dev.kampr.terminal.PaneSession

// Characters appear on screen because the pane echoed them. Nothing here keeps a local buffer,
// there is no compose box and there is no Send button.
// `onChord` is the way back out for the two keystrokes the pane must not be given. The grid is a
// canvas with nothing focusable on it, and in a browser the offscreen input holds the DOM focus for
// the life of a desk pane — so ctrl+shift+C is recognised where the keys land and routed to the
// caller, which is where the selection and the clipboard are.
@Composable
expect fun PaneTextInput(
    session: PaneSession,
    sink: InputSink,
    enabled: Boolean,
    onChord: (PaneChord) -> Unit,
    modifier: Modifier,
)
