package dev.kampr.shared

import dev.kampr.shared.net.Backoff
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.util.formatLatency
import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.util.relativeTime
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class ConnectionTest {
    @Test
    fun backoffGrowsAndCaps() {
        val backoff = Backoff()
        val delays = List(20) { backoff.next() }
        assertTrue(delays.first() <= 250)
        assertTrue(delays.last() <= 15_000)
        assertTrue(delays.drop(10).all { it >= 7_500 })
        backoff.reset()
        assertTrue(backoff.next() <= 250)
    }

    @Test
    fun endpointDerivesTheSocketUrl() {
        assertEquals("ws://192.168.1.24:8790/ws", Endpoint("http://192.168.1.24:8790").wsUrl)
        assertEquals("wss://kampr.example/ws", Endpoint("https://kampr.example/").wsUrl)
        assertTrue(Endpoint("https://kampr.example").secure)
    }

    @Test
    fun timesFormatTheWayTheArtboardsShowThem() {
        val at = parseIsoMillis("2026-08-20T13:44:02Z")!!
        assertEquals("now", relativeTime("2026-08-20T13:44:02Z", at))
        assertEquals("2m", relativeTime("2026-08-20T13:42:02Z", at))
        assertEquals("3h", relativeTime("2026-08-20T10:44:02Z", at))
        assertEquals("0.4 ms", formatLatency(0.4))
        assertEquals("38 ms", formatLatency(38.0))
        assertEquals("6 ms", formatLatency(6.0))
    }
}
