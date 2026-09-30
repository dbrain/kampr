package dev.kampr.terminal.input

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.KeyboardType
import dev.kampr.terminal.PaneSession

// `autoCorrectEnabled = false` only withholds the auto-correct flag, and Gboard corrects anyway —
// `winterview` went out as `winter view`. A URI field is one an IME does not correct or auto-space,
// and it still offers suggestions to tap. A password field would stop them too, and would have a
// screen reader speak every key as a dot; the visible-password class terminal emulators use is not
// in Compose 1.11.
@Composable
actual fun PaneTextInput(
    session: PaneSession,
    sink: InputSink,
    enabled: Boolean,
    onChord: (PaneChord) -> Unit,
    modifier: Modifier,
) = FieldTextInput(session, sink, enabled, onChord, modifier, KeyboardType.Uri)
