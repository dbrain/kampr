package dev.kampr.shared.net

import kotlinx.browser.document

@OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
private fun documentHidden(): Boolean = js("document.hidden")

// The listener is never unregistered: a Kotlin lambda handed to `addEventListener` is not
// guaranteed to be the same JS reference on the way back out, so a `removeEventListener` here
// would be a call that quietly does nothing.
actual fun watchForeground(onForeground: () -> Unit): ForegroundWatch {
    var watching = true
    document.addEventListener("visibilitychange", { if (watching && !documentHidden()) onForeground() })
    return ForegroundWatch { watching = false }
}
