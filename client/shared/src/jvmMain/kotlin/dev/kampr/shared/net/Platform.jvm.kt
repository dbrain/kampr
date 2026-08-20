package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import io.ktor.client.plugins.websocket.WebSockets

actual fun createHttpClient(): HttpClient = HttpClient(CIO) { install(WebSockets) }

actual fun defaultEndpoint(): Endpoint =
    Endpoint(System.getenv("KAMPR_NODE") ?: "http://127.0.0.1:8790", System.getenv("KAMPR_TOKEN"))

actual fun nowMillis(): Double = System.nanoTime() / 1_000_000.0

actual fun wallClockMillis(): Double = System.currentTimeMillis().toDouble()
