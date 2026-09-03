// VERBATIM from client/terminal/src/wasmJsMain/kotlin/dev/kampr/terminal/input/PaneTextInput.wasmJs.kt
function installDom(notify, chord) {
        (function () {
            if (globalThis.__kamprInput) {
                globalThis.__kamprInput.notify = notify;
                globalThis.__kamprInput.chord = chord;
                return;
            }
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

            var state = { el: el, queue: [], hold: false, notify: notify, chord: chord };
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
                    // Asked of `paneChords.kt` rather than decided here: this handler owns the
                    // browser's half — which key, which modifiers, and whether the page keeps its
                    // default — and the meaning of a chord is one table in common code.
                    var verdict = state.chord(
                        e.key + (e.ctrlKey ? 'c' : '') + (e.metaKey ? 'm' : '') + (e.shiftKey ? 's' : '')
                    );
                    // Taken: the selection has been copied or the clipboard read, here in Kotlin.
                    // Passed: a chord carrying the platform's command key, which is the browser's
                    // — `⌘T`, `⌘W`, `⌘L` — and must reach it with its default intact.
                    if (verdict === 'taken') { e.preventDefault(); return; }
                    if (verdict !== 'control') { return; }
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
}
