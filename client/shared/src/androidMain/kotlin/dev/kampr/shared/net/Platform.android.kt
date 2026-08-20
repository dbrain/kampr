package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.engine.okhttp.OkHttp
import io.ktor.client.plugins.websocket.WebSockets

actual fun createHttpClient(): HttpClient = HttpClient(OkHttp) { install(WebSockets) }

actual fun defaultEndpoint(): Endpoint = Endpoint("http://10.0.2.2:8790")

actual fun nowMillis(): Double = System.nanoTime() / 1_000_000.0

actual fun wallClockMillis(): Double = System.currentTimeMillis().toDouble()
