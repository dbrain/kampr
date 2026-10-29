package dev.kampr.terminal

import androidx.compose.foundation.text.BasicTextField
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.window.ComposeViewport
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.terminal.input.InputSink
import dev.kampr.terminal.input.Latches
import dev.kampr.terminal.input.PaneTextInput
import dev.kampr.terminal.input.focusInput
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.test.runTest
import kotlinx.coroutines.withContext
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.test.Test
import kotlin.test.assertEquals

@OptIn(ExperimentalWasmJsInterop::class)
private fun putStage(): org.w3c.dom.Element = js(
    "(function () { var d = document.createElement('div'); d.id = 'typing-stage'; document.body.appendChild(d); return d; })()",
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun removeStage() {
    js("(function () { var d = document.getElementById('typing-stage'); if (d) d.remove(); })()")
}

private suspend fun pause(ms: Long) = withContext(Dispatchers.Default) { delay(ms) }

private class Counting : PaneIo {
    var sent = 0

    override fun send(msg: ClientMsg) {
        sent++
    }

    override fun prefs(paneId: String) = PanePrefs()
}

private var mounted by mutableStateOf(false)
private var prompt by mutableStateOf(true)
private var composed by mutableStateOf("")

@OptIn(ExperimentalWasmJsInterop::class)
private fun typeIntoWhateverHasTheFocus(text: String) {
    js(
        """
        (function () {
            var a = document.activeElement;
            while (a && a.shadowRoot && a.shadowRoot.activeElement) a = a.shadowRoot.activeElement;
            if (!a) return;
            if (a.tagName === 'TEXTAREA' || a.tagName === 'INPUT') {
                a.dispatchEvent(new InputEvent('beforeinput', { data: text, inputType: 'insertText', bubbles: true, cancelable: true }));
                a.value = a.value + text;
                a.dispatchEvent(new InputEvent('input', { data: text, inputType: 'insertText', bubbles: true }));
            } else {
                a.dispatchEvent(new InputEvent('beforeinput', { data: text, inputType: 'insertText', bubbles: true, cancelable: true }));
            }
        })()
        """,
    )
}

@Composable
private fun PaneBesideComposer(session: PaneSession, sink: InputSink) {
    val ask = remember { FocusRequester() }
    LaunchedEffect(Unit) { ask.requestFocus() }
    PaneTextInput(session = session, sink = sink, enabled = !prompt, onChord = {}, modifier = Modifier)
    BasicTextField(value = composed, onValueChange = { composed = it }, modifier = Modifier.focusRequester(ask))
}

// The report as the operator lived it: the pane's input and the conversation composer on one page,
// a prompt held on the pane while they write, and the prompt clearing under them.
@OptIn(ExperimentalComposeUiApi::class)
class BrowserComposerTypingTest {
    @Test
    fun wordsTypedInTheComposerStayInTheComposerThroughAPromptClearing() = runTest {
        val io = Counting()
        val session = PaneSession(BROWSER_PANE)
        val sink = InputSink(BROWSER_PANE, io, Latches())
        mounted = true
        ComposeViewport(putStage()) { if (mounted) PaneBesideComposer(session, sink) }
        pause(100)

        prompt = false
        pause(600)
        typeIntoWhateverHasTheFocus("hi")
        pause(100)
        val received = composed
        val reachedTheTerminal = io.sent

        mounted = false
        pause(100)
        removeStage()
        focusInput(false)
        assertEquals("hi", received, "the composer never received what was typed")
        assertEquals(0, reachedTheTerminal, "what was typed in the composer reached the terminal")
    }
}
