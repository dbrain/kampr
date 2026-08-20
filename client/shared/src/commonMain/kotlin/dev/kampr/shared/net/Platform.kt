package dev.kampr.shared.net

import io.ktor.client.HttpClient

expect fun createHttpClient(): HttpClient

expect fun defaultEndpoint(): Endpoint

expect fun nowMillis(): Double

expect fun wallClockMillis(): Double
