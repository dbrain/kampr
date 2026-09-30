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

// Claude ramps a report that lands within 40 ms of the last one (#567), so a write sent too
// soon after the last moves further than the rows it was sized for. The frame that answers a write
// cannot predate it, so 40 ms after that frame is the earliest the next one is known to land fresh.
@OptIn(ExperimentalTestApi::class)
class FingerScrollPacingTest {
    private class Pump(val scroll: PaneScroll, val writes: MutableList<Long>)

    private fun ComposeUiTest.pump(): Pump {
        val writes = mutableListOf<Long>()
        lateinit var scroll: PaneScroll
        mainClock.autoAdvance = false
        setContent {
            val scope = rememberCoroutineScope()
            scroll = remember { PaneScroll(ScrollKeys.Wheel, scope = scope) { writes += mainClock.currentTime } }
        }
        mainClock.advanceTimeByFrame()
        return Pump(scroll, writes)
    }

    private fun ComposeUiTest.drag(pump: Pump, rows: Int) {
        runOnIdle { pump.scroll.refused(rows * 100f, step = 100f, col = 0, row = 0) }
        mainClock.advanceTimeBy(1, ignoreFrameDuration = true)
    }

    @Test
    fun theNextWriteWaitsOutClaudesRampAfterTheFrameThatAnsweredTheLast() = runComposeUiTest {
        val pump = pump()
        drag(pump, 5)
        assertEquals(1, pump.writes.size, "the first write of a gesture was made to wait")
        val first = pump.writes.single()

        drag(pump, 5)
        mainClock.advanceTimeBy(20, ignoreFrameDuration = true)
        runOnIdle { pump.scroll.answered() }
        val answered = mainClock.currentTime
        mainClock.advanceTimeBy(40, ignoreFrameDuration = true)
        assertEquals(1, pump.writes.size, "a write went out inside Claude's 40 ms ramp window")

        mainClock.advanceTimeBy(30, ignoreFrameDuration = true)
        assertEquals(2, pump.writes.size, "the frame came back and the travel still waited")
        val gap = pump.writes[1] - answered
        assertTrue(gap in 41..60, "the second write went ${gap}ms after the frame")
        assertTrue(pump.writes[1] - first < 150, "the frame was ignored and the ceiling waited out")
    }

    @Test
    fun aWriteNothingAnswersIsFollowedAtTheCeilingNotNever() = runComposeUiTest {
        val pump = pump()
        drag(pump, 5)
        drag(pump, 5)
        mainClock.advanceTimeBy(400, ignoreFrameDuration = true)
        assertEquals(2, pump.writes.size, "an unanswered write stalled the rest of the drag")
        assertTrue(pump.writes[1] - pump.writes[0] >= 150, "an unanswered write was not waited for")
    }
}
