package dev.kampr.terminal.input

import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.asPaddingValues
import androidx.compose.foundation.layout.ime
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.Dp
import dev.kampr.terminal.PaneSession

@Composable
actual fun PaneTextInput(
    session: PaneSession,
    sink: InputSink,
    enabled: Boolean,
    modifier: Modifier,
) = FieldTextInput(session, sink, enabled, modifier)

// The IME inset is the animated one and it is measured from the bottom of the window; the key row
// subtracts whatever sits below it, so a navigation bar does not become a static gap.
@Composable
actual fun rememberOskInset(): Dp = WindowInsets.ime.asPaddingValues().calculateBottomPadding()
