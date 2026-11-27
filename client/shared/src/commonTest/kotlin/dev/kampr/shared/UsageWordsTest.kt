package dev.kampr.shared

import dev.kampr.shared.util.dueWord
import dev.kampr.shared.util.limitsWord
import dev.kampr.shared.util.remainingSpan
import dev.kampr.shared.util.untilWord
import dev.kampr.shared.util.usageChip
import dev.kampr.shared.wire.Billing
import dev.kampr.shared.wire.Limits
import kotlinx.serialization.json.JsonPrimitive
import kotlin.test.Test
import kotlin.test.assertEquals

// 2026-11-27T12:00:00Z, read in UTC so the faces are the same on every machine.
private const val NOON = 1795780800000.0
private val UTC: (Double) -> Double = { 0.0 }

class UsageWordsTest {
    @Test
    fun aSpanIsCoarseUnlessItIsClose() {
        val cases = listOf(
            8_000.0 to "8s",
            (3 * 60 + 5) * 1000.0 to "3m 5s",
            (2 * 3600 + 29 * 60 + 57) * 1000.0 to "2h 29m",
            (26 * 3600) * 1000.0 to "1d 2h",
            -5_000.0 to "0s",
        )
        for ((millis, said) in cases) assertEquals(said, remainingSpan(millis), "$millis")
    }

    // The reset and the hold were drawn as the ISO stamp the source sent.
    @Test
    fun aMomentAheadIsSaidAsHowLongAndTheReadersClock() {
        assertEquals("in 2h 30m (14:30)", dueWord(NOON + 9_000_000, NOON, UTC))
        assertEquals("11:00", dueWord(NOON - 3_600_000, NOON, UTC))
        assertEquals("14:30 (in 2h 30m)", untilWord(NOON + 9_000_000, NOON, UTC))
        assertEquals("Max 5x · 72% · resets in 2h 30m (14:30)", limitsWord(Limits("Max 5x", JsonPrimitive(72), "2026-11-27T14:30:00Z"), NOON, UTC))
        assertEquals("resets soon", limitsWord(Limits(resetsAt = "soon"), NOON, UTC))
    }

    @Test
    fun theHeadersChipSaysUsageResetAndWhoPays() {
        val limits = Limits("Max 5x", JsonPrimitive(72), "2026-11-27T14:30:00Z")
        assertEquals("72% · resets 14:30 · Anthropic", usageChip(limits, Billing(provider = "Anthropic"), NOON, UTC))
        assertEquals("key work, rate-limited", usageChip(null, Billing(key = "work", rateLimited = true), NOON, UTC))
        assertEquals(null, usageChip(Limits(), Billing(), NOON, UTC))
    }
}
