package dev.kampr.terminal

import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.TimeMark
import kotlin.time.TimeSource

// The Compose test clock as a `TimeSource`, and `skew` to make a tick look as late as a busy main
// thread would have run it.
class TestClock(private val now: () -> Long) : TimeSource {
    var skew = 0L
    override fun markNow(): TimeMark = object : TimeMark {
        val at = now() + skew
        override fun elapsedNow() = (now() + skew - at).milliseconds
    }
}
