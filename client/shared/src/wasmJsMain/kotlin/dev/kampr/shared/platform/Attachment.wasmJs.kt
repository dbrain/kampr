package dev.kampr.shared.platform

import dev.kampr.shared.net.DownloadSink
import kotlin.io.encoding.Base64
import kotlin.js.ExperimentalWasmJsInterop

// The bytes come out of an authorised `fetch`, so there is no URL a browser could be pointed at to
// download them a second time — a bare `<a href download>` cannot carry a bearer header. An object
// URL over a Blob is the only form of "download" available here. Each chunk becomes a Blob of its
// own as it arrives, and a Blob of Blobs is one the browser may keep on disk rather than in the
// page's heap, which is what lets a file larger than the tab's memory be saved at all.
//
// No state crosses between `js()` bodies except through the page, because each one compiles to a
// standalone function with no closure of its own.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsBegin(key: String): Unit = js("(window.__kamprDownloads = window.__kamprDownloads || {})[key] = []")

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsChunk(key: String, base64: String): Unit = js(
    """
    (function () {
      var binary = atob(base64);
      var buffer = new Uint8Array(binary.length);
      for (var i = 0; i < binary.length; i++) buffer[i] = binary.charCodeAt(i);
      window.__kamprDownloads[key].push(new Blob([buffer]));
    })()
    """,
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsFinish(key: String, name: String, mime: String): Boolean = js(
    """
    (function () {
      var parts = window.__kamprDownloads[key];
      delete window.__kamprDownloads[key];
      try {
        var url = URL.createObjectURL(new Blob(parts, { type: mime }));
        var anchor = document.createElement('a');
        anchor.href = url;
        anchor.download = name;
        document.body.appendChild(anchor);
        anchor.click();
        document.body.removeChild(anchor);
        setTimeout(function () { URL.revokeObjectURL(url); }, 60000);
        return true;
      } catch (e) {
        return false;
      }
    })()
    """,
)

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsAbort(key: String): Unit = js("delete (window.__kamprDownloads || {})[key]")

private var downloads = 0

private class BlobSink(private val name: String, private val mime: String) : DownloadSink {
    private val key = "d${downloads++}"

    init {
        jsBegin(key)
    }

    fun put(chunk: ByteArray, length: Int) = jsChunk(key, Base64.encode(chunk, 0, length))

    fun done(): String? = if (runCatching { jsFinish(key, name, mime) }.getOrDefault(false)) name else null

    override suspend fun write(chunk: ByteArray, length: Int) = put(chunk, length)

    override suspend fun finish(): String? = done()

    override suspend fun abort() = jsAbort(key)
}

actual fun saveToDevice(name: String, mime: String?, bytes: ByteArray): String? = runCatching {
    BlobSink(name, mime ?: "application/octet-stream").run {
        put(bytes, bytes.size)
        done()
    }
}.getOrNull()

actual fun openDownload(name: String, mime: String?): DownloadSink? = runCatching { BlobSink(name, mime ?: "application/octet-stream") }.getOrNull()
