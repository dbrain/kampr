package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.websocket.WebSockets
import okhttp3.OkHttpClient

actual fun createHttpClient(): HttpClient = HttpClient(OkHttp) {
    engine { preconfigured = OkHttpClient.Builder().socketFactory(NoDelaySockets).build() }
    install(WebSockets)
}

actual fun defaultEndpoint(): Endpoint? = null

actual fun deviceName(): String = listOf(android.os.Build.MANUFACTURER, android.os.Build.MODEL)
    .filter { it.isNotBlank() }
    .joinToString(" ")
    .ifBlank { "Android" }

actual fun nowMillis(): Double = System.nanoTime() / 1_000_000.0

actual fun wallClockMillis(): Double = System.currentTimeMillis().toDouble()
