package dev.kampr.terminal.input

import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.InputTransformation
import androidx.compose.foundation.text.input.TextFieldBuffer
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.rememberTextFieldState
import androidx.compose.foundation.text.input.setTextAndPlaceCursorAtEnd
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.snapshotFlow
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
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardCapitalization
import dev.kampr.terminal.PaneSession

// A run of spaces kept in front of the cursor so the field is never empty. A soft keyboard chooses
// between a key event and a deletion for backspace depending on whether it can see anything there,
// and only one of the two ever carried a payload; with padding both do, and the key path consumes
// the event before the field can delete as well. A space rather than a zero-width space because
// every IME treats a space as a word boundary and so will not pull it into a composing region.
private const val PAD_LENGTH = 64

// Below this the padding is restored, above it the field is emptied back to bare padding. Both
// rewrite text the IME did not write and cost a restartInput, so both sit far from ordinary
// typing: the floor is dozens of backspaces past an empty command line, the ceiling a command
// nobody types.
private const val PAD_FLOOR = 16
private const val PAD_CEILING = 1024

private val PAD = " ".repeat(PAD_LENGTH)

// The IME-backed capture used everywhere except the browser: a zero-size field whose edits are
// diffed rather than displayed. Autocapitalise and autocorrect are off because a terminal is not
// prose.
//
// The field is never handed back text other than the text the IME itself put there. A field whose
// value the app answers with something else is answered in turn with InputMethodManager
// .restartInput, and a restarted Gboard drops to its letters page — a digit that will not stay
// typed — and abandons the input connection, discarding any keystroke already in flight on it.
// This one emptied itself after every commit, so that was every keystroke. The state therefore
// lives in the buffer the IME edits rather than in a value round-tripped through recomposition,
// and what goes to the pane is the difference that buffer itself reports rather than a diff
// against a copy this file keeps. Padding is restored only through `state.edit`, which the input
// transformation does not see, so no space in it is ever typed at the pane.
@Composable
fun FieldTextInput(
    session: PaneSession,
    sink: InputSink,
    enabled: Boolean,
    modifier: Modifier,
) {
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    val state = rememberTextFieldState(PAD, TextRange(PAD_LENGTH))
    val diff = remember(sink) { DiffToPane(sink) }

    // Focus alone does not raise the IME on Android; the controller is what actually shows it.
    LaunchedEffect(session.focusRequests, session.keyboardOpen, enabled) {
        if (enabled && session.keyboardOpen) {
            runCatching { focus.requestFocus() }
            keyboard?.show()
        } else {
            keyboard?.hide()
        }
    }

    LaunchedEffect(state) {
        snapshotFlow { state.text.length }.collect { length ->
            if (length < PAD_FLOOR || length > PAD_CEILING) state.setTextAndPlaceCursorAtEnd(PAD)
        }
    }

    BasicTextField(
        state = state,
        modifier = modifier
            .focusRequester(focus)
            .onPreviewKeyEvent { event -> enabled && handleKeyEvent(event, sink) },
        enabled = enabled,
        inputTransformation = diff,
        lineLimits = TextFieldLineLimits.SingleLine,
        keyboardOptions = KeyboardOptions(
            capitalization = KeyboardCapitalization.None,
            autoCorrectEnabled = false,
            imeAction = ImeAction.None,
        ),
    )
}

private class DiffToPane(private val sink: InputSink) : InputTransformation {
    override fun TextFieldBuffer.transformInput() {
        emitDiff(originalText, asCharSequence(), sink)
    }
}

private fun emitDiff(previous: CharSequence, current: CharSequence, sink: InputSink) {
    var shared = 0
    while (shared < previous.length && shared < current.length && previous[shared] == current[shared]) shared++
    val removed = previous.length - shared
    if (removed > 0) sink.raw(Esc.BACKSPACE.repeat(removed))
    if (current.length > shared) sink.type(current.subSequence(shared, current.length).toString())
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
