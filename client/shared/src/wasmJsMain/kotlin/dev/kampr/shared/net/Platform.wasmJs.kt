package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.engine.js.Js
import io.ktor.client.plugins.websocket.WebSockets
import kotlinx.browser.window

actual fun createHttpClient(): HttpClient = HttpClient(Js) { install(WebSockets) }

actual fun defaultEndpoint(): Endpoint =
    Endpoint(window.location.protocol + "//" + window.location.host)

actual fun nowMillis(): Double = window.performance.now()

@OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
private fun dateNow(): Double = js("Date.now()")

actual fun wallClockMillis(): Double = dateNow()
