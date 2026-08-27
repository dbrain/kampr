package dev.kampr.shared

import dev.kampr.shared.util.clockFace
import dev.kampr.shared.util.isoIsZoned
import dev.kampr.shared.util.parseIsoMillis
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val HOUR = 3_600_000.0
private val AEST = 10 * HOUR

private fun at(text: String): Double = requireNotNull(parseIsoMillis(text)) { "unparsed: $text" }

class TimeTest {
    // Every harness Kampr reads stamps UTC and says so, so the offset in the string is the one
    // fact that lets a clock face be drawn at all — and reading it is what makes two stamps of
    // the same instant, written in different zones, the same instant here.
    @Test
    fun aStampCarriesItsOwnOffsetAndIsReadInIt() {
        val utc = at("2026-08-24T05:00:00.000Z")
        assertEquals(utc, at("2026-08-24T15:00:00+10:00"), "a +10:00 stamp is not the same instant")
        assertEquals(utc, at("2026-08-24T00:00:00-05:00"), "a -05:00 stamp is not the same instant")
        assertEquals(utc, at("2026-08-24T15:00:00+1000"), "a colonless offset is not read")
    }

    @Test
    fun aStampThatNamesNoOffsetSaysSo() {
        assertTrue(isoIsZoned("2026-08-24T05:00:00.000Z"))
        assertTrue(isoIsZoned("2026-08-24T15:00:00+10:00"))
        assertFalse(isoIsZoned("2026-08-24T15:00:00"))
        assertFalse(isoIsZoned("2026-08-24T15:00:00.123"))
        assertFalse(isoIsZoned(null))
    }

    // Today is bare, the last week wears its day, anything older wears its date. A year that is
    // not this one is the only case that spends the extra characters.
    @Test
    fun aFaceSaysAsMuchOfTheDateAsTheReaderNeeds() {
        val now = at("2026-08-24T05:00:00Z")
        val cases = listOf(
            "2026-08-24T05:00:00Z" to "15:00",
            "2026-08-24T00:04:00Z" to "10:04",
            "2026-08-22T22:00:00Z" to "Sun 08:00",
            "2026-08-19T03:30:00Z" to "Wed 13:30",
            "2026-08-17T23:30:00Z" to "Tue 09:30",
            "2026-08-16T03:30:00Z" to "16 Aug 13:30",
            "2024-12-31T13:30:00Z" to "31 Dec 2024 23:30",
        )
        for ((stamp, want) in cases) {
            assertEquals(want, clockFace(at(stamp), now, AEST), "$stamp in AEST")
        }
    }

    // The whole point of a face over an age: it is drawn from the instant, so it does not move
    // while the reader looks at it, and a device whose own clock is minutes out still draws it
    // right. Only the *bucket* — today, this week, older — reads the device clock at all.
    @Test
    fun aFaceDoesNotMoveWhenTheDeviceClockIsWrong() {
        val stamp = at("2026-08-24T05:00:00Z")
        val truth = at("2026-08-24T06:00:00Z")
        for (skew in listOf(-120_000.0, 0.0, 120_000.0)) {
            assertEquals("15:00", clockFace(stamp, truth + skew, AEST), "skewed $skew ms")
        }
    }

    // A transcript routinely spans a daylight-saving move, which is why the offset is asked for
    // per instant rather than once for the device.
    @Test
    fun eachInstantIsDrawnInTheOffsetThatWasInForceForIt() {
        val now = at("2026-08-24T05:00:00Z")
        assertEquals("16 Aug 13:30", clockFace(at("2026-08-16T03:30:00Z"), now, AEST))
        assertEquals("16 Aug 12:30", clockFace(at("2026-08-16T03:30:00Z"), now, AEST - HOUR))
    }
}
