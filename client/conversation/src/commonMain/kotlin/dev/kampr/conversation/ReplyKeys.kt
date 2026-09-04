package dev.kampr.conversation

import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isMetaPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key

// Readline's line editing, in the one box on this screen that is a neighbour of a terminal. The
// operator asked for it in the words they use at a shell — "on bash i can ctrl+A ctrl+E ctrl+U" —
// and the reply box is where a prompt for the agent in that same pane gets written, so the hands
// arrive already holding those keys.
//
// **ctrl+A is line start here and not select-all**, which is what the platform mapping gives it
// everywhere else in Compose. That is the trade the ask makes and it is worth naming: a box beside
// a terminal is a line being edited, the whole of it is four sentences at most, and a drag or a
// triple-click still selects it. ctrl+C, ctrl+V, ctrl+X, ctrl+Z and ctrl+Y are left exactly where
// the platform put them.
enum class LineKey { Start, End, KillToStart, KillToEnd, KillWord }

// The modifiers are read strictly: a chord carrying alt or meta is somebody else's, and shift with
// one of these is not a readline key at all — ctrl+shift+Z is the platform's redo and taking it
// here would swallow it.
fun lineKeyFor(event: KeyEvent): LineKey? {
    if (!event.isCtrlPressed || event.isAltPressed || event.isMetaPressed || event.isShiftPressed) return null
    return when (event.key) {
        Key.A -> LineKey.Start
        Key.E -> LineKey.End
        Key.U -> LineKey.KillToStart
        Key.K -> LineKey.KillToEnd
        Key.W -> LineKey.KillWord
        else -> null
    }
}

// What the field is asked to do, as one range: the text in `from until to` goes, and the caret
// lands on `from`. A motion is the empty range at wherever it moved to, so both halves of every
// one of these keys are the same edit and the field applies them the same way.
data class LineEdit(val from: Int, val to: Int)

// A selection is collapsed rather than ignored: a kill takes it with whatever else it takes, and a
// motion leaves from the end of it that it is travelling away from. Nothing here reads a soft
// keyboard's composing region, because nothing that draws one sends these chords.
fun lineEdit(key: LineKey, text: CharSequence, min: Int, max: Int): LineEdit = when (key) {
    LineKey.Start -> lineStart(text, min).let { LineEdit(it, it) }
    LineKey.End -> lineEnd(text, max).let { LineEdit(it, it) }
    LineKey.KillToStart -> LineEdit(lineStart(text, min), max)
    LineKey.KillToEnd -> LineEdit(min, lineEnd(text, max))
    LineKey.KillWord -> LineEdit(wordStart(text, min), max)
}

// The line, not the box: shift and return writes a second one here, so a reply is often several
// and a ctrl+A that went to the top of all of them would be an editor's Home, not a shell's.
private fun lineStart(text: CharSequence, at: Int): Int {
    var i = at.coerceIn(0, text.length)
    while (i > 0 && text[i - 1] != '\n') i--
    return i
}

private fun lineEnd(text: CharSequence, at: Int): Int {
    var i = at.coerceIn(0, text.length)
    while (i < text.length && text[i] != '\n') i++
    return i
}

// unix-word-rubout: the whitespace-delimited word behind the caret, and the run of spaces that
// leads to it. Whitespace includes the newline, so this crosses a line break exactly as it does at
// a shell whose command has one in it.
private fun wordStart(text: CharSequence, at: Int): Int {
    var i = at.coerceIn(0, text.length)
    while (i > 0 && text[i - 1].isWhitespace()) i--
    while (i > 0 && !text[i - 1].isWhitespace()) i--
    return i
}
