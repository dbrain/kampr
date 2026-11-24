package dev.kampr.terminal

import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.Latches
import kotlin.test.Test
import kotlin.test.assertEquals

// A browser's paste reaches the sink through `type`, framed by the page, so the mark is read off
// the payload rather than off the call that made it: only a write that is one whole paste is one
// the node may frame again for a program that never turned bracketed paste on.
class MarkingAPasteTest {
    private class Recording : PaneIo {
        val sent = mutableListOf<ClientMsg.InputText>()
        override fun send(msg: ClientMsg) {
            if (msg is ClientMsg.InputText) sent += msg
        }
        override fun prefs(paneId: String) = PanePrefs()
    }

    @Test
    fun onlyAWriteThatIsOneWholePasteIsMarked() {
        val io = Recording()
        val sink = InputSink("p", io, Latches())
        sink.paste("s3cr3t")
        sink.type("\u001b[200~a\nb\u001b[201~")
        sink.type("ls")
        sink.type("\u001b[200~a\u001b[201~x")
        assertEquals(
            listOf(true, true, false, false),
            io.sent.map { it.paste },
            io.sent.joinToString { it.text },
        )
        assertEquals("\u001b[200~s3cr3t\u001b[201~", io.sent[0].text)
    }
}
