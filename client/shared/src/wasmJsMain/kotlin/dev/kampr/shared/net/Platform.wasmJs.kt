package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.engine.js.Js
import io.ktor.client.plugins.websocket.WebSockets
import kotlinx.browser.window

actual fun createHttpClient(): HttpClient = HttpClient(Js) { install(WebSockets) }

actual fun defaultEndpoint(): Endpoint? =
    Endpoint(window.location.protocol + "//" + window.location.host)

// A browser name, not a fingerprint: the node already records the full user agent, and what the
// device list needs is something a person can tell apart at a glance.
@OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
private fun jsDeviceName(): String = js(
    """
    (function () {
      var ua = navigator.userAgent || '';
      var browser = /Firefox\//.test(ua) ? 'Firefox'
        : /Edg\//.test(ua) ? 'Edge'
        : /OPR\//.test(ua) ? 'Opera'
        : /Chrome\//.test(ua) ? 'Chrome'
        : /Safari\//.test(ua) ? 'Safari'
        : 'Browser';
      var platform = /Android/.test(ua) ? 'Android'
        : /iPhone|iPad|iPod/.test(ua) ? 'iOS'
        : /Mac OS X/.test(ua) ? 'macOS'
        : /Windows/.test(ua) ? 'Windows'
        : /Linux/.test(ua) ? 'Linux'
        : '';
      return platform ? browser + ' on ' + platform : browser;
    })()
    """
)

actual fun deviceName(): String = runCatching { jsDeviceName() }.getOrDefault("Browser")

actual fun nowMillis(): Double = window.performance.now()

@OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
private fun dateNow(): Double = js("Date.now()")

actual fun wallClockMillis(): Double = dateNow()
