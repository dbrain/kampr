package dev.kampr.shared.platform

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.await
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.js.JsString
import kotlin.js.Promise

// `readText` is the narrow half of the clipboard API and the only half every browser implements:
// `read()`, which the paste listener in `FilePick.wasmJs.kt` exists to avoid, is refused outright
// by Firefox and Safari. This one they answer — behind a confirmation of the browser's own, drawn
// over the page, because a page reading the clipboard unasked is the thing that permission is for.
//
// Which is also why it can come back empty for a reason that is not an empty clipboard: a refusal,
// an insecure origin, or a gesture the browser no longer considers live. The caller says so.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsReadText(): Promise<JsString?> = js(
    """
    (function () {
      if (!navigator.clipboard || !navigator.clipboard.readText) return Promise.resolve(null);
      return navigator.clipboard.readText().catch(function () { return null; });
    })()
    """
)

actual suspend fun clipboardText(): String? = try {
    jsReadText().await<JsString?>()?.toString()?.takeIf { it.isNotEmpty() }
} catch (cancelled: CancellationException) {
    throw cancelled
} catch (_: Throwable) {
    null
}
