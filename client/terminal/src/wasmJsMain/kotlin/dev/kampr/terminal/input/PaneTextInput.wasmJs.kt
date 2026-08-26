package dev.kampr.terminal.input

import androidx.compose.foundation.layout.Box
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.withFrameNanos
import androidx.compose.ui.Modifier
import dev.kampr.shared.platform.touchBrowser
import dev.kampr.terminal.PaneSession
import kotlin.js.ExperimentalWasmJsInterop

// Android soft keyboards report keyCode 229 for every printable key, so keydown is unusable as
// the text path in a browser. The readable input is an offscreen contenteditable driven through
// beforeinput / input / composition*, diffed against what it last held. keydown is kept
// only for the keys a hardware keyboard sends and an IME never does.
@OptIn(ExperimentalWasmJsInterop::class)
private fun installDom(notify: () -> Unit) {
    js(
        """
        (function () {
            if (globalThis.__kamprInput) { globalThis.__kamprInput.notify = notify; return; }
            var el = document.createElement('div');
            el.setAttribute('contenteditable', 'plaintext-only');
            el.setAttribute('autocapitalize', 'off');
            el.setAttribute('autocorrect', 'off');
            el.setAttribute('autocomplete', 'off');
            el.setAttribute('spellcheck', 'false');
            el.setAttribute('inputmode', 'text');
            el.setAttribute('enterkeyhint', 'enter');
            el.setAttribute('aria-label', 'terminal input');
            el.style.cssText = 'position:fixed;left:0;bottom:0;width:1px;height:1px;padding:0;' +
                'border:0;outline:none;opacity:0;color:transparent;caret-color:transparent;' +
                'background:transparent;overflow:hidden;z-index:-1;';
            document.body.appendChild(el);

            var state = { el: el, queue: [], hold: false, notify: notify };
            globalThis.__kamprInput = state;

            var composed = '';
            var handled = false;

            function push(text) { if (text) state.queue.push(text); }

            function listen(type, handler) {
                el.addEventListener(type, function (e) {
                    handler(e);
                    if (state.queue.length && state.notify) state.notify();
                });
            }

            function diff(previous, current) {
                var shared = 0;
                while (shared < previous.length && shared < current.length &&
                       previous[shared] === current[shared]) shared++;
                var out = '';
                for (var i = shared; i < previous.length; i++) out += '\u007f';
                return out + current.slice(shared);
            }

            listen('compositionstart', function () { composed = ''; });
            listen('compositionupdate', function (e) {
                var data = e.data || '';
                push(diff(composed, data));
                composed = data;
            });
            listen('compositionend', function (e) {
                var data = e.data || '';
                push(diff(composed, data));
                composed = '';
                el.textContent = '';
            });

            listen('beforeinput', function (e) {
                if (e.isComposing) return;
                var kind = e.inputType;
                handled = true;
                if (kind === 'insertFromPaste') {
                    var pasted = e.data || (e.dataTransfer ? e.dataTransfer.getData('text') : '');
                    if (pasted) push('\u001b[200~' + pasted + '\u001b[201~');
                } else if (kind === 'insertText' || kind === 'insertReplacementText') {
                    push(e.data || '');
                } else if (kind === 'deleteContentBackward' || kind === 'deleteWordBackward') {
                    push('\u007f');
                } else if (kind === 'deleteContentForward') {
                    push('\u001b[3~');
                } else if (kind === 'insertLineBreak' || kind === 'insertParagraph') {
                    push('\r');
                } else {
                    handled = false;
                    return;
                }
                e.preventDefault();
            });

            listen('input', function (e) {
                if (e.isComposing) return;
                var current = el.textContent || '';
                if (!handled && current.length) push(current);
                handled = false;
                if (current.length) el.textContent = '';
            });

            var named = {
                'Escape': '\u001b', 'Tab': '\t', 'Enter': '\r', 'Backspace': '\u007f',
                'Delete': '\u001b[3~', 'Insert': '\u001b[2~',
                'Home': '\u001b[H', 'End': '\u001b[F',
                'PageUp': '\u001b[5~', 'PageDown': '\u001b[6~',
                'ArrowUp': '\u001b[A', 'ArrowDown': '\u001b[B',
                'ArrowRight': '\u001b[C', 'ArrowLeft': '\u001b[D',
                'F1': '\u001bOP', 'F2': '\u001bOQ', 'F3': '\u001bOR', 'F4': '\u001bOS',
                'F5': '\u001b[15~', 'F6': '\u001b[17~', 'F7': '\u001b[18~', 'F8': '\u001b[19~',
                'F9': '\u001b[20~', 'F10': '\u001b[21~', 'F11': '\u001b[23~', 'F12': '\u001b[24~'
            };

            function modified(sequence, e) {
                if (!e.ctrlKey && !e.altKey && !e.shiftKey) return sequence;
                var code = 1;
                if (e.shiftKey) code += 1;
                if (e.altKey) code += 2;
                if (e.ctrlKey) code += 4;
                var last = sequence[sequence.length - 1];
                if (sequence.length === 3 && (sequence[1] === '[' || sequence[1] === 'O')) {
                    return '\u001b[1;' + code + last;
                }
                if (last === '~') return sequence.slice(0, -1) + ';' + code + '~';
                return sequence;
            }

            listen('keydown', function (e) {
                if (e.isComposing || e.keyCode === 229) return;
                var sequence = named[e.key];
                if (sequence) {
                    if (sequence.length > 1) sequence = modified(sequence, e);
                    else if (e.altKey) sequence = '\u001b' + sequence;
                    push(sequence);
                    e.preventDefault();
                    return;
                }
                if ((e.ctrlKey || e.metaKey) && e.key && e.key.length === 1) {
                    var lower = e.key.toLowerCase();
                    var body = null;
                    if (lower >= 'a' && lower <= 'z') {
                        body = String.fromCharCode(lower.charCodeAt(0) - 96);
                    } else if (lower === ' ' || lower === '@') { body = '\u0000'; }
                    else if (lower === '[') { body = '\u001b'; }
                    else if (lower === '\\') { body = '\u001c'; }
                    else if (lower === ']') { body = '\u001d'; }
                    else if (lower === '_' || lower === '-') { body = '\u001f'; }
                    if (body !== null) {
                        push(e.altKey ? '\u001b' + body : body);
                        e.preventDefault();
                    }
                }
            });

        })()
        """
    )
}

@OptIn(ExperimentalWasmJsInterop::class)
internal fun drainInput(): String =
    js("(function () { var s = globalThis.__kamprInput; if (!s || !s.queue.length) return ''; var out = s.queue.join(''); s.queue.length = 0; return out; })()")

// The queue used to be emptied from a Compose frame loop, and `withFrameNanos` is
// `requestAnimationFrame`. A browser stops calling that back for a hidden tab, so the drain parked
// with the tab and whatever was in the array arrived in one lump when the tab came back, with
// nothing saying so; and even on a visible tab every keystroke waited up to a frame before it was
// sent, which is the only latency Kampr still owns: #273 puts herdr's own share at 1-3 ms.
//
// So a listener hands over before it returns and delivery has no clock at all. The array stays,
// because the ordering is the array: several events land in one turn of the event loop,
// `compositionupdate` interleaves with `beforeinput`, and the order they arrived in is what the
// operator typed. Only the emptying moved.
//
// A keystroke that lands with no sink standing stays queued and goes to the next one the instant
// it stands up, which is what makes the handover between two panes lossless. Nothing waits here
// for a frame any more, so a keystroke that does not reach the pane is one the socket could not
// carry — and that one is counted by `PaneState.undelivered` and badged, where an operator can
// see it. This array was upstream of every counter in the client.
private var deliverTo: ((String) -> Unit)? = null

private fun flushInput() {
    val deliver = deliverTo ?: return
    val pending = drainInput()
    if (pending.isNotEmpty()) deliver(pending)
}

internal fun deliverInputTo(deliver: ((String) -> Unit)?) {
    flushInput()
    deliverTo = deliver
    flushInput()
}

internal fun installInput() = installDom(::flushInput)

// `touchBrowser` and not the key row's reading, because these are two questions off one browser
// and they want different thresholds. Here the question is whether focusing would raise a soft
// keyboard over the pane, and only a coarse pointer has one to raise — so a browser that reports
// no input device at all is held, not left alone. The key row asks whether there is a hardware
// keyboard, which nothing has answered in that case, and it offers the row. Sharing one answer
// meant a browser reporting nothing had to be either a desk or a phone for both, and it is
// neither.
//
// What this one means: focusing is what raises the keys, so a touch browser is left alone until
// the operator asks — a pane that opened underneath the keyboard is the regression this guards.
// A desk browser has no keys to raise, so the input holds the focus for as long as a pane is on
// screen, which is the only thing that makes a hardware keyboard reach the pane at all.

internal fun holdsInput(enabled: Boolean, keyboardAsked: Boolean, touch: Boolean): Boolean =
    enabled && (keyboardAsked || !touch)

@OptIn(ExperimentalWasmJsInterop::class)
internal fun focusInput(open: Boolean) {
    js(
        """
        (function () {
            var s = globalThis.__kamprInput;
            if (!s) return;
            s.hold = open;
            if (open) { s.el.textContent = ''; s.el.focus({ preventScroll: true }); }
            else { s.el.blur(); }
        })()
        """
    )
}

// DOM focus is one slot for the whole page, and the pane's claim on it used to be asserted exactly
// once — from a `LaunchedEffect` keyed on the keyboard cap, a focus request, a settled gesture and
// `enabled`. None of those move when something else on the page takes the slot, so the claim was
// never renewed: the conversation composer, a button, any page chrome took the keyboard and kept
// it, and the pane went on painting frames from the desk while every keystroke went to the page.
// A pane that shows output and takes no keys, with nothing complaining, is the shape this exists
// to stop.
//
// Renewed per frame rather than on `focusout`, because the strand is usually the *second* move.
// The composer takes the focus legitimately, and then hands it back to the body when it closes —
// and no event fires on an element that lost the focus some time ago, so there is nothing to hang
// a listener on. A standing claim has to be re-checked to be worth anything.
//
// Standing down for a live text field is what keeps the composer usable: Compose's own text input
// is a real `<textarea>` in this DOM, so an editable element holding the focus is always someone
// who needs the keys more than the pane does. Everything else — a button, the body, nothing at
// all — is not typing, and the pane takes the slot back.
@OptIn(ExperimentalWasmJsInterop::class)
internal fun reclaimInputFocus() {
    js(
        """
        (function () {
            var s = globalThis.__kamprInput;
            if (!s || !s.hold) return;
            var active = document.activeElement;
            if (active === s.el) return;
            if (active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA' ||
                           active.isContentEditable)) return;
            s.el.focus({ preventScroll: true });
        })()
        """
    )
}

@Composable
actual fun PaneTextInput(
    session: PaneSession,
    sink: InputSink,
    enabled: Boolean,
    modifier: Modifier,
) {
    LaunchedEffect(Unit) { installInput() }
    val touch = remember { touchBrowser() }
    LaunchedEffect(session.keyboardOpen, session.focusRequests, session.surfaceSettled, enabled) {
        focusInput(holdsInput(enabled, session.keyboardOpen, touch))
    }
    DisposableEffect(sink, enabled) {
        deliverInputTo(if (enabled) sink::type else null)
        onDispose { deliverInputTo(null) }
    }
    // A frame is the wrong clock for delivering a keystroke and the only one there is for renewing
    // a focus claim: nothing fires when the slot is lost to something that took it a while ago, so
    // the claim has to be re-checked, and a hidden tab has no focus to lose.
    LaunchedEffect(enabled) {
        if (!enabled) return@LaunchedEffect
        while (true) {
            withFrameNanos { }
            reclaimInputFocus()
        }
    }
    Box(modifier)
}
