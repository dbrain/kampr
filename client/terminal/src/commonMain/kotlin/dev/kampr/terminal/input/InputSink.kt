package dev.kampr.terminal.input

import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg

private const val PASTE_START = "\u001b[200~"
private const val PASTE_END = "\u001b[201~"

private const val SHIFTED_UNSHIFTED = "`1234567890-=[]\\;',./"
private const val SHIFTED_SHIFTED = "~!@#\$%^&*()_+{}|:\"<>?"

// Everything leaves as input.text. b64 exists on the wire for control characters, but every
// sequence the key row produces is UTF-8-safe, so text is the one path (#9).
class InputSink(private val paneId: String, private val io: PaneIo, val latches: Latches) {
    fun raw(text: String) {
        if (text.isEmpty()) return
        io.send(ClientMsg.InputText(paneId, text))
    }

    // Probe #9: pane.send_text writes raw bytes with no bracketed-paste framing of its own, so a
    // multi-line paste would execute line by line in a shell unless Kampr brackets it here.
    fun paste(text: String) {
        if (text.isEmpty()) return
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
