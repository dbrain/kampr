package dev.kampr.shared.net

import kotlinx.coroutines.await
import kotlin.js.ExperimentalWasmJsInterop
import kotlin.js.JsBoolean
import kotlin.js.Promise

// `boot.js` holds the deferred event, because it fires long before this module exists.
@OptIn(ExperimentalWasmJsInterop::class)
private fun jsAvailable(): Boolean = js("!!(window.kamprInstall && window.kamprInstall.available())")

@OptIn(ExperimentalWasmJsInterop::class)
private fun jsPrompt(): Promise<JsBoolean> = js("window.kamprInstall.prompt()")

private class BrowserInstallPrompt : InstallPrompt {
    override val available: Boolean get() = runCatching { jsAvailable() }.getOrDefault(false)

    override suspend fun prompt(): Boolean =
        runCatching { jsPrompt().await().toBoolean() }.getOrDefault(false)
}

actual fun createInstallPrompt(): InstallPrompt = BrowserInstallPrompt()
