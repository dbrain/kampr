package dev.kampr.terminal.input

import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.input.InputTransformation
import androidx.compose.foundation.text.input.TextFieldBuffer
import androidx.compose.foundation.text.input.TextFieldLineLimits
import androidx.compose.foundation.text.input.placeCursorAtEnd
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
import kotlinx.coroutines.flow.drop

// What the buffer is *for*, beyond never being empty: a soft keyboard reads the line back out of
// it — the composing region, the suggestion strip and the correction it will type if the operator
// accepts one all come from what it can see in front of the cursor. So it has to follow the pane's
// line rather than only ever grow. It did only ever grow: the key path consumes a backspace, so
// the pane lost a character and the editor kept it, and Gboard went on offering a correction to a
// word that had not been on the pane for three keystrokes.
//
// Two rules keep them in step, and both write through `state.edit`, which the input transformation
// does not see — so nothing here is ever typed at the pane. A backspace takes one character off.
// Anything else that reaches the pane without passing through this buffer — the Enter that ran the
// command, an escape, an arrow, a cap on the key row, a paste — puts it back to bare padding,
// because the caret has moved off the line the buffer was mirroring and no part of it is true any
// more.
//
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

    // Only when there is a line to let go of. Re-seeding the field is the one write here the IME
    // did not ask for, it costs a `restartInput`, and a restarted Gboard drops to its letters page
    // and abandons any keystroke in flight on the connection — so an arrow pressed at an empty
    // prompt, which is most of them, must not pay for one.
    val letGo = { if (state.text.toString() != PAD) state.setTextAndPlaceCursorAtEnd(PAD) }

    LaunchedEffect(sink, state) {
        snapshotFlow { sink.offField }.drop(1).collect { letGo() }
    }

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
            .onPreviewKeyEvent { event ->
                if (!enabled) {
                    false
                } else {
                    when (handleKeyEvent(event, sink)) {
                        null -> false
                        Echo.Erase -> {
                            state.edit {
                                if (length > 0) replace(length - 1, length, "")
                                placeCursorAtEnd()
                            }
                            true
                        }
                        Echo.Drop -> {
                            letGo()
                            true
                        }
                        Echo.Kept -> true
                    }
                }
            },
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

// What the buffer owes the keystroke that was just handled: `Erase` one character, `Drop` the line
// it was mirroring, `Kept` for a key that was consumed and sent nothing. `null` is the event this
// file did not take, which the field is then free to apply itself.
private enum class Echo { Erase, Drop, Kept }

// **A soft keyboard delivers its action key as a real key event and carries no modifier state with
// it.** So a latch armed on the key row was dropped on exactly the chord that needs one — alt+enter,
// which is how an agent's prompt box takes a newline, and which submitted the message instead. The
// special keys therefore ride the latches here the way the row's own caps already ride them through
// `InputSink.press`.
//
// Characters are deliberately left on the hardware modifiers alone: `InputSink.type` owns the IME's
// character path and decorates them there, and reading the latch in both places would spend it on a
// keystroke that has not been sent yet.
private fun handleKeyEvent(event: KeyEvent, sink: InputSink): Echo? {
    if (event.type != KeyEventType.KeyDown) return null
    val ctrl = event.isCtrlPressed || sink.latches.ctrl.active()
    val alt = event.isAltPressed || sink.latches.alt.active()
    val shift = event.isShiftPressed || sink.latches.shift.active()
    val function = functionKeys.indexOf(event.key)
    if (function >= 0) {
        sink.latches.consume()
        sink.raw(Esc.modified(Esc.function(function + 1), ctrl, alt, shift))
        return Echo.Drop
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
        sink.latches.consume()
        sink.raw(payload)
        return if (event.key == Key.Backspace && !ctrl && !alt) Echo.Erase else Echo.Drop
    }
    if (!event.isCtrlPressed && !event.isAltPressed) return null
    val code = event.utf16CodePoint
    if (code in 1..0xFFFF) {
        val ch = code.toChar()
        val body = if (event.isCtrlPressed) Esc.control(ch) else ch.toString()
        if (body != null) {
            sink.raw(if (event.isAltPressed) Esc.ESCAPE + body else body)
            return Echo.Drop
        }
    }
    return if (event.isCtrlPressed) Echo.Kept else null
}
