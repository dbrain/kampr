package dev.kampr.shared.platform

import dev.kampr.shared.net.DirectUpload
import dev.kampr.shared.net.UploadSource
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.await
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.longOrNull
import kotlin.io.encoding.Base64
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.js.JsString
import kotlin.js.Promise

// The chosen `File` stays on the page and only its name and size cross into Kotlin: the bytes are
// read off the disk by the browser as they are sent, never copied into the wasm heap.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsPickUpload(): Promise<JsString?> = js(
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
          var box = window.__kamprUploads = window.__kamprUploads || { next: 0 };
          var key = 'u' + (box.next++);
          box[key] = { file: file, sent: 0, xhr: null };
          done(JSON.stringify({ key: key, name: file.name, size: file.size }));
        });
        input.click();
      });
    })()
    """,
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsSlice(key: String, start: Double, end: Double): Promise<JsString> = js(
    """
    window.__kamprUploads[key].file.slice(start, end).arrayBuffer().then(function (buffer) {
      var bytes = new Uint8Array(buffer), s = '';
      for (var i = 0; i < bytes.length; i++) s += String.fromCharCode(bytes[i]);
      return btoa(s);
    })
    """,
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsPost(key: String, url: String, bearer: String): Promise<JsString?> = js(
    """
    new Promise(function (resolve) {
      var entry = window.__kamprUploads[key];
      var xhr = new XMLHttpRequest();
      entry.xhr = xhr;
      xhr.open('POST', url);
      if (bearer) xhr.setRequestHeader('Authorization', 'Bearer ' + bearer);
      xhr.setRequestHeader('Content-Type', 'application/octet-stream');
      xhr.upload.onprogress = function (event) { entry.sent = event.loaded; };
      xhr.onload = function () { resolve(JSON.stringify({ status: xhr.status, body: xhr.responseText || '' })); };
      xhr.onerror = function () { resolve(null); };
      xhr.onabort = function () { resolve(null); };
      xhr.send(entry.file);
    })
    """,
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsSent(key: String): Double = js("(window.__kamprUploads[key] || { sent: 0 }).sent")

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsForget(key: String): Unit = js(
    """
    (function () {
      var entry = (window.__kamprUploads || {})[key];
      if (entry && entry.xhr && entry.xhr.readyState !== 4) entry.xhr.abort();
      if (window.__kamprUploads) delete window.__kamprUploads[key];
    })()
    """,
)

private class BrowserFile(private val key: String, override val name: String, override val size: Long) :
    UploadSource,
    DirectUpload {
    private var offset = 0L

    @OptIn(ExperimentalWasmJsInterop::class)
    override suspend fun read(into: ByteArray): Int {
        if (offset >= size) return -1
        val end = minOf(size, offset + into.size)
        val bytes = Base64.decode(jsSlice(key, offset.toDouble(), end.toDouble()).await<JsString>().toString())
        bytes.copyInto(into)
        offset += bytes.size
        return bytes.size
    }

    override suspend fun close() = jsForget(key)

    @OptIn(ExperimentalWasmJsInterop::class)
    override suspend fun post(url: String, bearer: String?, progress: (Long) -> Unit): DirectUpload.Answer? = try {
        coroutineScope {
            val watch = launch {
                while (true) {
                    progress(jsSent(key).toLong())
                    delay(250)
                }
            }
            val answer = jsPost(key, url, bearer.orEmpty()).await<JsString?>()?.toString()
            watch.cancel()
            progress(jsSent(key).toLong())
            answer?.let(::answerOf)
        }
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Throwable) {
        null
    } finally {
        jsForget(key)
    }
}

private fun JsonObject.field(key: String) = this[key] as? JsonPrimitive

private fun answerOf(text: String): DirectUpload.Answer? {
    val obj = runCatching { Json.parseToJsonElement(text) as? JsonObject }.getOrNull() ?: return null
    val status = obj.field("status")?.intOrNull ?: return null
    return DirectUpload.Answer(status, obj.field("body")?.contentOrNull.orEmpty())
}

@OptIn(ExperimentalWasmJsInterop::class)
actual suspend fun pickUpload(): UploadSource? {
    val answer = runCatching { jsPickUpload().await<JsString?>() }.getOrNull()?.toString() ?: return null
    val obj = runCatching { Json.parseToJsonElement(answer) as? JsonObject }.getOrNull() ?: return null
    val key = obj.field("key")?.contentOrNull ?: return null
    val name = obj.field("name")?.contentOrNull?.takeIf(String::isNotBlank) ?: "upload"
    val size = obj.field("size")?.longOrNull ?: return null
    return BrowserFile(key, name, size)
}
