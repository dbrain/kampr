package dev.kampr.terminal.input

import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.key.utf16CodePoint
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import dev.kampr.terminal.PaneSession

// The IME-backed capture used everywhere except the browser: a zero-size field whose value is
// diffed rather than displayed. Autocapitalise and autocorrect are off because a terminal is not
// prose, and the field is emptied after every commit so a backspace against an empty buffer still
// arrives as a key event rather than being swallowed.
@Composable
fun FieldTextInput(
    session: PaneSession,
    sink: InputSink,
    enabled: Boolean,
    modifier: Modifier,
) {
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    var value by remember { mutableStateOf(TextFieldValue("")) }

    // Focus alone does not raise the IME on Android; the controller is what actually shows it.
    LaunchedEffect(session.focusRequests, session.keyboardOpen, enabled) {
        if (enabled && session.keyboardOpen) {
            runCatching { focus.requestFocus() }
            keyboard?.show()
        } else {
            keyboard?.hide()
        }
    }

    BasicTextField(
        value = value,
        onValueChange = { next ->
            if (enabled) {
                emitDiff(value.text, next.text, sink)
                value = if (next.composition != null) next else TextFieldValue("")
            }
        },
        modifier = modifier
            .focusRequester(focus)
            .onPreviewKeyEvent { event -> enabled && handleKeyEvent(event, sink) },
        singleLine = true,
        keyboardOptions = KeyboardOptions(
            capitalization = KeyboardCapitalization.None,
            autoCorrectEnabled = false,
            imeAction = ImeAction.None,
        ),
    )
}

private fun emitDiff(previous: String, current: String, sink: InputSink) {
    if (previous == current) return
    var shared = 0
    while (shared < previous.length && shared < current.length && previous[shared] == current[shared]) shared++
    val removed = previous.length - shared
    if (removed > 0) sink.raw(Esc.BACKSPACE.repeat(removed))
    if (current.length > shared) sink.type(current.substring(shared))
}

private val functionKeys = listOf(
    Key.F1, Key.F2, Key.F3, Key.F4, Key.F5, Key.F6,
    Key.F7, Key.F8, Key.F9, Key.F10, Key.F11, Key.F12,
)

private fun sequenceFor(key: Key): String? = when (key) {
    Key.Escape -> Esc.ESCAPE
    Key.Tab -> Esc.TAB
    Key.Enter, Key.NumPadEnter -> Esc.ENTER
    Key.Backspace -> Esc.BACKSPACE
    Key.Delete -> Esc.DELETE
    Key.Insert -> Esc.INSERT
    Key.MoveHome -> Esc.HOME
    Key.MoveEnd -> Esc.END
    Key.PageUp -> Esc.PAGE_UP
    Key.PageDown -> Esc.PAGE_DOWN
    Key.DirectionUp -> Esc.UP
    Key.DirectionDown -> Esc.DOWN
    Key.DirectionLeft -> Esc.LEFT
    Key.DirectionRight -> Esc.RIGHT
    else -> null
}

private fun handleKeyEvent(event: KeyEvent, sink: InputSink): Boolean {
    if (event.type != KeyEventType.KeyDown) return false
    val ctrl = event.isCtrlPressed
    val alt = event.isAltPressed
    val shift = event.isShiftPressed
    val function = functionKeys.indexOf(event.key)
    if (function >= 0) {
        sink.raw(Esc.modified(Esc.function(function + 1), ctrl, alt, shift))
        return true
    }
    val mapped = sequenceFor(event.key)
    if (mapped != null) {
        val payload = if (mapped.startsWith(Esc.ESCAPE) && mapped.length > 1) {
            Esc.modified(mapped, ctrl, alt, shift)
        } else if (alt) {
            Esc.ESCAPE + mapped
        } else {
            mapped
        }
        sink.raw(payload)
        return true
    }
    if (!ctrl && !alt) return false
    val code = event.utf16CodePoint
    if (code in 1..0xFFFF) {
        val ch = code.toChar()
        val body = if (ctrl) Esc.control(ch) else ch.toString()
        if (body != null) {
            sink.raw(if (alt) Esc.ESCAPE + body else body)
            return true
        }
    }
    return ctrl
}
