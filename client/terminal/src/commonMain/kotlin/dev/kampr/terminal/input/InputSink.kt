package dev.kampr.terminal.input

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.terminal.guard.SubmitGuard

private const val PASTE_START = "\u001b[200~"
private const val PASTE_END = "\u001b[201~"

// A paste with no line terminator lands on the input line and is caught at the Enter that follows
// it; only a paste that carries its own submit can run on arrival, so only that one is inspected
// here. Guarding both would confirm the same command twice.
private fun pasteBody(text: String): String? {
    if (!text.startsWith(PASTE_START)) return null
    val end = text.indexOf(PASTE_END)
    val body = if (end < 0) text.substring(PASTE_START.length) else text.substring(PASTE_START.length, end)
    return body.takeIf { it.contains('\n') || it.contains('\r') }
}

private const val SHIFTED_UNSHIFTED = "`1234567890-=[]\\;',./"
private const val SHIFTED_SHIFTED = "~!@#\$%^&*()_+{}|:\"<>?"

// Everything leaves as input.text. b64 exists on the wire for control characters, but every
// sequence the key row produces is UTF-8-safe, so text is the one path (#9).
class InputSink(
    private val paneId: String,
    private val io: PaneIo,
    val latches: Latches,
    private val guard: SubmitGuard? = null,
) {
    // Every byte that reaches the pane without passing through the IME's own buffer: a cap on the
    // key row, a paste, a submit the guard held and then let go. The hidden field mirrors the
    // pane's input line so a soft keyboard can read it back and correct against it, and after one
    // of these it no longer does — so it is told to let go of the line rather than to go on
    // offering a word for a caret that has moved.
    var offField by mutableStateOf(0)
        private set

    private fun elsewhere() {
        offField++
    }

    // Every byte that actually left for the pane, counted. Two surfaces owe an answer to "the
    // operator has just typed into this pane" and neither can see the send otherwise: the viewport,
    // which goes back to following the caret because typing is a request to be shown what you
    // typed, and the handover line, whose whole statement is about a composer the next keystroke
    // has already moved on from.
    var sends by mutableIntStateOf(0)
        private set

    private fun emit(text: String) {
        if (text.isEmpty()) return
        sends++
        io.send(ClientMsg.InputText(paneId, text))
    }

    // The submit is the hook, not the keystroke: by the time a whole command has been typed it is
    // already in the PTY, so the only thing that can still be held back is the Enter that runs it.
    fun raw(text: String) {
        if (text.isEmpty()) return
        if (guard == null) {
            emit(text)
            return
        }
        guard.clear()
        val pasted = pasteBody(text)
        if (pasted != null) {
            if (!guard.hold(pasted, text, paste = true)) emit(text)
            return
        }
        val submit = text.indexOfFirst { it == '\r' || it == '\n' }
        if (submit < 0) {
            emit(text)
            return
        }
        // Anything ahead of the Enter in the same payload is ordinary typing and goes now, but the
        // pane has not echoed it yet, so it is appended to the line the guard reads.
        val typed = text.take(submit)
        emit(typed)
        val rest = text.substring(submit)
        if (!guard.hold(guard.commandLine() + typed, rest, paste = false)) emit(rest)
    }

    fun confirmed(payload: String) {
        elsewhere()
        emit(payload)
    }

    // Probe #9: pane.send_text writes raw bytes with no bracketed-paste framing of its own, so a
    // multi-line paste would execute line by line in a shell unless Kampr brackets it here.
    fun paste(text: String) {
        if (text.isEmpty()) return
        elsewhere()
        raw(PASTE_START + text + PASTE_END)
    }

    fun press(cap: KeyCap) {
        val ctrl = latches.ctrl.active()
        val alt = latches.alt.active()
        val shift = latches.shift.active()
        val payload = if (cap.csi) {
            Esc.modified(cap.send, ctrl, alt, shift)
        } else {
            decorate(cap.send, ctrl, alt, shift)
        }
        latches.consume()
        elsewhere()
        raw(payload)
    }

    // Characters arrive already composed by the platform; a latch only ever decorates the first
    // one, because the operator armed it for one keystroke. A payload that already starts with a
    // control byte came from a hardware key rather than an IME and carries its own modifiers.
    fun type(text: String) {
        if (text.isEmpty()) return
        if (text[0].code < 0x20 || text[0] == '\u007f') {
            raw(text)
            return
        }
        if (!latches.ctrl.active() && !latches.alt.active() && !latches.shift.active()) {
            raw(text)
            return
        }
        val head = decorate(
            text.take(1), latches.ctrl.active(), latches.alt.active(), latches.shift.active(),
        )
        latches.consume()
        raw(head + text.drop(1))
    }

    private fun decorate(text: String, ctrl: Boolean, alt: Boolean, shift: Boolean): String {
        if (text.isEmpty()) return text
        var out = text
        if (shift && out.length == 1) out = shifted(out[0]).toString()
        if (ctrl && out.length == 1) out = Esc.control(out[0]) ?: out
        if (alt) out = Esc.ESCAPE + out
        return out
    }

    private fun shifted(ch: Char): Char {
        if (ch.isLetter()) return ch.uppercaseChar()
        val index = SHIFTED_UNSHIFTED.indexOf(ch)
        return if (index >= 0) SHIFTED_SHIFTED[index] else ch
    }
}
