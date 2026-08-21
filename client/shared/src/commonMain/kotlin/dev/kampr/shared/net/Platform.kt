package dev.kampr.shared.net

import io.ktor.client.HttpClient

expect fun createHttpClient(): HttpClient

// Null means "this device cannot know". Only a client the node itself served can derive an
// address from where it is running; an installed app cannot, and a guess there is an error
// message on first launch for something the operator has not done wrong yet.
expect fun defaultEndpoint(): Endpoint?

// What this device calls itself in the node's device list. Every paired device was named "device"
// because nothing ever sent one, so a phone and a laptop were indistinguishable in the one list
// revocation is decided from.
expect fun deviceName(): String

expect fun nowMillis(): Double

expect fun wallClockMillis(): Double
