package dev.kampr.terminal.input

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import dev.kampr.terminal.PaneSession

@Composable
actual fun PaneTextInput(
    session: PaneSession,
    sink: InputSink,
    enabled: Boolean,
    modifier: Modifier,
) = FieldTextInput(session, sink, enabled, modifier)
