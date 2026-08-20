package dev.kampr.shared.net

import kotlin.math.min
import kotlin.random.Random

class Backoff(
    private val initialMs: Long = 250,
    private val factor: Double = 1.8,
    private val maxMs: Long = 15_000,
) {
    private var attempt = 0

    fun reset() {
        attempt = 0
    }

    fun next(): Long {
        val base = min(maxMs.toDouble(), initialMs * pow(factor, attempt)).toLong()
        attempt++
        return base / 2 + Random.nextLong(base / 2 + 1)
    }

    private fun pow(base: Double, exp: Int): Double {
        var out = 1.0
        repeat(exp) { out *= base }
        return out
    }
}
