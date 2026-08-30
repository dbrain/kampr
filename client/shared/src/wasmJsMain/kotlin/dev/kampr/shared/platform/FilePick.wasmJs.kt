package dev.kampr.shared.platform

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.await
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlin.io.encoding.Base64
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.js.JsString
import kotlin.js.Promise

actual val filePickAvailable: Boolean = true

// The bytes come back as base64 because that is the only shape that crosses this boundary without
// a typed-array marshaller, and it is the shape the wire wants anyway. The input element is thrown
// away with the choice: a browser fires no event at all when a picker is dismissed, so an element
// left in the document is a listener that never runs again.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsPick(): Promise<JsString?> = js(
    """
    (function () {
      return new Promise(function (resolve) {
        var input = document.createElement('input');
        input.type = 'file';
        input.style.display = 'none';
        document.body.appendChild(input);
        var done = function (value) {
          if (input.parentNode) document.body.removeChild(input);
          resolve(value);
        };
        input.addEventListener('cancel', function () { done(null); });
        input.addEventListener('change', function () {
          var file = input.files && input.files[0];
          if (!file) return done(null);
          var reader = new FileReader();
          reader.onerror = function () { done(null); };
          reader.onload = function () {
            var bytes = new Uint8Array(reader.result), s = '';
            for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
            done(JSON.stringify({ name: file.name, mime: file.type, b64: btoa(s) }));
          };
          reader.readAsArrayBuffer(file);
        });
        input.click();
      });
    })()
    """
)

// **The `paste` event is the only way a page is given clipboard bytes without asking permission**,
// and it is the only one that treats a screenshot on the clipboard and a file copied in a file
// manager as the same thing — the first arrives as an item of kind `file` with no name, the second
// as an entry in `files` with one. `navigator.clipboard.read()` can reach the first, behind a
// permission prompt Firefox and Safari refuse outright, and cannot reach the second at all.
//
// The listener is installed once for the life of the page and is deliberately **not** removed with
// the composable that reads from it. A listener added and removed as panes come and go is a
// listener that is absent for the paste that arrives while a view is being switched; what is
// scoped to the composable is the *waiter*, so an unread file simply sits in the queue.
//
// It is a capture-phase listener so that it sees the event before Compose's own, and it calls
// `preventDefault` **only** when it took files — a file copied in a file manager also puts its
// name on the clipboard as text, and pasting that name into the reply box is not what the operator
// asked for. Nothing here stops propagation: a clipboard carrying both is still Compose's to read.
//
// No state crosses between `js()` bodies except through the page, because each one compiles to a
// standalone function with no closure of its own.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsNextPaste(): Promise<JsString?> = js(
    """
    (function () {
      var box = window.__kamprPaste;
      if (!box) {
        box = window.__kamprPaste = { ready: [], waiting: [] };
        var deliver = function (value) {
          var waiter = box.waiting.shift();
          if (waiter) waiter(value); else box.ready.push(value);
        };
        window.addEventListener('paste', function (event) {
          var data = event.clipboardData;
          if (!data) return;
          var files = [];
          for (var i = 0; i < data.items.length; i++) {
            var item = data.items[i];
            if (item.kind !== 'file') continue;
            var file = item.getAsFile();
            if (file) files.push(file);
          }
          if (!files.length) return;
          event.preventDefault();
          files.forEach(function (file) {
            var reader = new FileReader();
            reader.onerror = function () {};
            reader.onload = function () {
              var bytes = new Uint8Array(reader.result), s = '';
              for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
              deliver(JSON.stringify({ name: file.name, mime: file.type, b64: btoa(s) }));
            };
            reader.readAsArrayBuffer(file);
          });
        }, true);
      }
      return new Promise(function (resolve) {
        if (box.ready.length) resolve(box.ready.shift());
        else box.waiting.push(resolve);
      });
    })()
    """
)

private fun pickedFrom(answer: String): PickedFile? {
    val obj = runCatching { Json.parseToJsonElement(answer) as? JsonObject }.getOrNull() ?: return null
    fun field(key: String) = (obj[key] as? JsonPrimitive)?.contentOrNull
    val bytes = runCatching { Base64.decode(field("b64").orEmpty()) }.getOrNull() ?: return null
    return bytes.takeIf { it.isNotEmpty() }?.let {
        PickedFile(field("name")?.takeIf(String::isNotBlank), field("mime")?.takeIf(String::isNotBlank), it)
    }
}

@OptIn(ExperimentalWasmJsInterop::class)
actual suspend fun pickFile(): PickedFile? {
    val answer = runCatching { jsPick().await<JsString?>() }.getOrNull()?.toString() ?: return null
    return pickedFrom(answer)
}

// A file the browser could not read is dropped rather than delivered, so the loop here is for the
// one that is malformed after all of that — it goes back to waiting rather than reporting the end
// of the clipboard, because one unreadable paste must not be the last one this pane will take.
@OptIn(ExperimentalWasmJsInterop::class)
actual suspend fun pastedFile(): PickedFile? {
    while (true) {
        val answer = runCatching { jsNextPaste().await<JsString?>() }
            .getOrElse { if (it is CancellationException) throw it else return null }
            ?: return null
        pickedFrom(answer.toString())?.let { return it }
    }
}

// The window listener above already sees every paste on the page, wherever the focus is, so there
// is nothing for a modifier to add here.
@Composable
actual fun Modifier.acceptsPastedFiles(): Modifier = this
