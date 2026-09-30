package dev.kampr.terminal

import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.test.ComposeUiTest
import androidx.compose.ui.test.ExperimentalTestApi
import androidx.compose.ui.test.runComposeUiTest
import dev.kampr.terminal.input.PaneScroll
import dev.kampr.terminal.input.ScrollKeys
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// Claude ramps a report that lands within 40 ms of the last one (#567), and the ramp carries from
// one write to the next when they land that close (#569). So a write is either sent inside
// the window, sized from where the last one left the ramp, or far enough outside it that no link
// can pull it back in — never in between, where jitter decides what it moves.
@OptIn(ExperimentalTestApi::class)
class FingerScrollPacingTest {
    private class Write(val at: Long, val text: String)

    private class Pump(val scroll: PaneScroll, val writes: MutableList<Write>, val clock: TestClock)

    private fun ComposeUiTest.pump(): Pump {
        val writes = mutableListOf<Write>()
        val clock = TestClock { mainClock.currentTime }
        lateinit var scroll: PaneScroll
        mainClock.autoAdvance = false
        setContent {
            val scope = rememberCoroutineScope()
            scroll = remember {
                PaneScroll(ScrollKeys.Wheel, scope = scope, clock = clock) { writes += Write(mainClock.currentTime, it) }
            }
        }
        mainClock.advanceTimeByFrame()
        return Pump(scroll, writes, clock)
    }

    private fun ComposeUiTest.drag(pump: Pump, rows: Int) {
        runOnIdle { pump.scroll.refused(rows * 100f, step = 100f, col = 0, row = 0) }
    }

    private fun ComposeUiTest.tick(ms: Long) = mainClock.advanceTimeBy(ms, ignoreFrameDuration = true)

    private class Run(val moved: Int, val writes: List<Write>, val lift: Long) {
        val gaps = writes.zipWithNext { a, b -> b.at - a.at }
        val trail = writes.last().at - lift
        val medianGap = gaps.sorted().let { if (it.isEmpty()) -1 else it[it.size / 2] }
    }

    // A finger reporting every 8 ms (120 Hz) at an even rate, frames answering each write
    // `answerMs` later, and each write reaching Claude `latency(i)` after it left — in order,
    // as one socket delivers them.
    private fun ComposeUiTest.finger(
        rows: Int,
        ms: Int,
        answerMs: Long = 10,
        latency: (Int) -> Long = { 0L },
    ): Run {
        val pump = pump()
        val start = mainClock.currentTime
        var asked = 0
        var answered = 0
        for (t in 1..ms + 600) {
            if (t <= ms && t % 8 == 0 || t == ms) {
                val want = rows * t / ms
                if (want > asked) drag(pump, want - asked)
                asked = want
            }
            val now = mainClock.currentTime
            if (pump.writes.drop(answered).any { it.at + answerMs <= now }) {
                answered = pump.writes.size
                runOnIdle { pump.scroll.answered() }
            }
            tick(1)
        }
        val claude = ClaudeWheel()
        var arrived = Long.MIN_VALUE
        pump.writes.forEachIndexed { i, w ->
            arrived = maxOf(arrived, w.at + latency(i))
            claude.write(arrived, w.text)
        }
        return Run(claude.rows, pump.writes.toList(), start + ms)
    }

    private fun Run.neverInsideTheJitterBand(what: String) {
        val band = gaps.filter { it in 25..59 }
        assertTrue(band.isEmpty(), "$what: writes sent inside the band jitter decides: $band")
    }

    // The table #571 ran against a real Claude, run against the judge instead.
    @Test
    fun aFingerMovesClaudeExactlyAsFarAsItTravelledAndStopsWithIt() = runComposeUiTest {
        for ((rows, ms) in listOf(30 to 250, 100 to 250, 60 to 1000, 150 to 600, 20 to 1000, 200 to 1000)) {
            val run = finger(rows, ms)
            assertEquals(rows, run.moved, "$rows rows in $ms ms moved Claude ${run.moved}")
            assertTrue(run.trail <= 100, "$rows rows in $ms ms: the last write went ${run.trail} ms after the lift")
            run.neverInsideTheJitterBand("$rows rows in $ms ms")
        }
    }

    // The smoothness. #568's pump let a write out 45 ms after the frame that answered the last,
    // ~14 a second; a finger moving fast enough to keep the ramp carried is written every tick.
    @Test
    fun aFastFingerIsWrittenEveryTickRatherThanEveryRoundTrip() = runComposeUiTest {
        val run = finger(rows = 150, ms = 600)
        assertTrue(run.medianGap in 16..20, "writes went out every ${run.medianGap} ms: ${run.gaps}")
        assertTrue(run.writes.size >= 30, "150 rows in 600 ms went out as ${run.writes.size} writes")
    }

    // The jitter budget: writes carried 16 ms apart may arrive up to 24 ms apart and still carry,
    // so a link that moves each write by anything from 0 to 16 ms still lands the finger exactly.
    @Test
    fun aLinkThatJittersInsideTheBudgetStillLandsTheFingerExactly() = runComposeUiTest {
        val jitter = listOf(0L, 16L, 3L, 11L, 0L, 16L, 7L, 16L, 0L, 9L)
        for ((rows, ms) in listOf(100 to 250, 150 to 600, 60 to 1000)) {
            val run = finger(rows, ms) { i -> jitter[i % jitter.size] }
            assertEquals(rows, run.moved, "$rows rows in $ms ms over a jittery link moved ${run.moved}")
        }
    }

    @Test
    fun theFirstWriteOfAGestureIsNotMadeToWait() = runComposeUiTest {
        val pump = pump()
        drag(pump, 5)
        tick(1)
        assertEquals(1, pump.writes.size, "the first write of a gesture was made to wait")
    }

    // A tick the dispatcher ran late is not a tick inside the window: the clock says how long it
    // really was, and a write that might land past Claude's reset is not sized as if it carried.
    @Test
    fun aTickThatRanLateWaitsOutTheResetInsteadOfCarrying() = runComposeUiTest {
        val pump = pump()
        drag(pump, 5)
        tick(1)
        drag(pump, 20)
        pump.clock.skew += 10
        tick(16)
        assertEquals(1, pump.writes.size, "a write 26 ms after the last was sized as if it carried")
        runOnIdle { pump.scroll.answered() }
        tick(80)
        assertEquals(2, pump.writes.size, "the travel never went out after the reset")
        assertTrue(pump.writes[1].at - pump.writes[0].at >= 60, "the fresh write went inside the jitter band")
    }

    @Test
    fun aWriteNothingAnswersIsFollowedAtTheCeilingNotNever() = runComposeUiTest {
        val pump = pump()
        drag(pump, 5)
        tick(1)
        tick(30)
        drag(pump, 1)
        tick(400)
        assertEquals(2, pump.writes.size, "an unanswered write stalled the rest of the drag")
        assertTrue(pump.writes[1].at - pump.writes[0].at >= 150, "an unanswered write was not waited for")
    }
}
