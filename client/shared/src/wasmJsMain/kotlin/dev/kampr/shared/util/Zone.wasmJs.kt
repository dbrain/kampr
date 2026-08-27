package dev.kampr.shared.util

@OptIn(kotlin.js.ExperimentalWasmJsInterop::class)
private fun offsetOf(atMillis: Double): Double = js("-new Date(atMillis).getTimezoneOffset() * 60000")

actual fun localOffsetMillis(atMillis: Double): Double = offsetOf(atMillis)
