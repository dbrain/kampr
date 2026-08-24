package dev.kampr.shared.platform

import kotlin.io.encoding.Base64
import kotlin.js.ExperimentalWasmJsInterop

// The bytes came out of an authorised `fetch`, so there is no URL a browser could be pointed at to
// download them a second time — a bare `<a href download>` cannot carry a bearer header. An object
// URL over the bytes already in hand is the only form of "download" available here.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsSave(name: String, mime: String, base64: String): Boolean = js(
    """
    (function () {
      try {
        var binary = atob(base64);
        var buffer = new Uint8Array(binary.length);
        for (var i = 0; i < binary.length; i++) buffer[i] = binary.charCodeAt(i);
        var url = URL.createObjectURL(new Blob([buffer], { type: mime }));
        var anchor = document.createElement('a');
        anchor.href = url;
        anchor.download = name;
        document.body.appendChild(anchor);
        anchor.click();
        document.body.removeChild(anchor);
        setTimeout(function () { URL.revokeObjectURL(url); }, 10000);
        return true;
      } catch (e) {
        return false;
      }
    })()
    """
)

actual fun saveToDevice(name: String, mime: String?, bytes: ByteArray): String? {
    val encoded = runCatching { Base64.encode(bytes) }.getOrNull() ?: return null
    val handed = runCatching { jsSave(name, mime ?: "application/octet-stream", encoded) }.getOrDefault(false)
    return if (handed) name else null
}
