package dev.kampr.terminal.input

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
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

// The key row docks above the on-screen keyboard rather than replacing it, so dictation, glide
// typing and language switching all keep working.
@Composable
expect fun rememberOskInset(): Dp
