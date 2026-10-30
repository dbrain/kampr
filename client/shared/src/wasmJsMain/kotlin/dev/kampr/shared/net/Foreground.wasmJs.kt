package dev.kampr.shared.net

import kotlinx.browser.document
import kotlinx.browser.window

@OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
private fun documentHidden(): Boolean = js("document.hidden")

// The listener is never unregistered: a Kotlin lambda handed to `addEventListener` is not
// guaranteed to be the same JS reference on the way back out, so a `removeEventListener` here
// would be a call that quietly does nothing.
// A focused window is somebody coming back to it as much as a shown tab is: a desk with the browser
// on one screen is never hidden, only left for something else.
actual fun watchForeground(onChange: (shown: Boolean) -> Unit): ForegroundWatch {
    var watching = true
    document.addEventListener("visibilitychange", { if (watching) onChange(!documentHidden()) })
    window.addEventListener("focus", { if (watching && !documentHidden()) onChange(true) })
    return ForegroundWatch { watching = false }
}
