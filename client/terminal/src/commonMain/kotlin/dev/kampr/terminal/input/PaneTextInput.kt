package dev.kampr.terminal.input

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import dev.kampr.terminal.PaneSession

// Characters appear on screen because the pane echoed them. Nothing here keeps a local buffer,
// there is no compose box and there is no Send button.
@Composable
expect fun PaneTextInput(
    session: PaneSession,
    sink: InputSink,
    enabled: Boolean,
    modifier: Modifier,
)
