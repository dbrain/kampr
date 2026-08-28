package dev.kampr.shared.platform

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

@OptIn(ExperimentalWasmJsInterop::class)
actual suspend fun pickFile(): PickedFile? {
    val answer = runCatching { jsPick().await<JsString?>() }.getOrNull()?.toString() ?: return null
    val obj = runCatching { Json.parseToJsonElement(answer) as? JsonObject }.getOrNull() ?: return null
    fun field(key: String) = (obj[key] as? JsonPrimitive)?.contentOrNull
    val bytes = runCatching { Base64.decode(field("b64").orEmpty()) }.getOrNull() ?: return null
    return bytes.takeIf { it.isNotEmpty() }?.let {
        PickedFile(field("name")?.takeIf(String::isNotBlank), field("mime")?.takeIf(String::isNotBlank), it)
    }
}
