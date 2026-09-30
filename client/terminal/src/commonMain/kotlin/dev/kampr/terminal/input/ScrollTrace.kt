package dev.kampr.terminal.input

import dev.kampr.terminal.bench.emitBench
import dev.kampr.terminal.bench.platformLabel
import kotlin.math.abs
import kotlin.time.TimeMark
import kotlin.time.TimeSource

// Read once, at the platform entry point, before anything composes — the same switch the bench is
// behind (`bench` intent extra, `?bench`, `KAMPR_BENCH`). Deliberately not snapshot state: nothing
// re-reads it, and a pane that is not being traced pays one boolean per report for it.
var scrollTracing: Boolean = false

// A gesture is over when nothing has been sent for this long. The wheel has no gesture at all —
// no down, no up — so a boundary the *input* defines is the only one both paths have.
private const val IDLE_GAP_MS = 400L

// What one gesture asked a program for, and how long it waited.
//
// A pane whose harness holds the alternate screen keeps no ring (#387), so every row above the
// screen is fetched by sending the program a scroll report and waiting for it to repaint. The
// operator's report is that a finger on such a pane chugs where a wheel at the desk does not, and
// the two numbers that tell those apart — reports sent per gesture, and the wait between a report
// going out and the frame it caused coming back — are both invisible from outside the app.
// `dumpsys gfxinfo` counts frames the app drew, which conflates a slow round trip with a screen
// that did not change; measured on a live pane it swung between 0% and 12.5% jank on identical
// idle windows and gave two opposite answers for the same gesture.
class ScrollTrace(
    // Taken at construction rather than read per call, so a test can trace without touching a
    // global and two panes cannot disagree about whether they are being traced.
    private val on: Boolean = scrollTracing,
    private val clock: TimeSource = TimeSource.Monotonic,
    private val emit: (String) -> Unit = ::emitBench,
) {
    private var started = clock.markNow()
    private var lastSent = clock.markNow()
    private var span = 0L
    private var waitingSince: TimeMark? = null
    private var open = false
    private var reports = 0
    private var writes = 0
    private var carried = 0
    private var rows = 0
    private var travel = 0
    private var frames = 0
    private val waits = mutableListOf<Long>()

    // `rows` is what the program is expected to move for them, which is the number to hold against
    // `travel`: a finger that travelled 60 rows and moved the pane 20 is the defect this exists for.
    fun sent(keys: ScrollKeys, reports: Int, rows: Int, carried: Boolean = false) {
        if (!on) return
        if (open && lastSent.elapsedNow().inWholeMilliseconds > IDLE_GAP_MS) flush(keys)
        if (!open) {
            open = true
            started = clock.markNow()
        }
        lastSent = clock.markNow()
        span = started.elapsedNow().inWholeMilliseconds
        this.reports += reports
        this.rows += rows
        writes++
        if (carried) this.carried++
        // Only the first unanswered write starts the clock: the wait being measured is the round
        // trip, and writes two and three of a burst are queued behind the same repaint.
        if (waitingSince == null) waitingSince = clock.markNow()
    }

    fun travelled(rows: Int) {
        if (on) travel += abs(rows)
    }

    // Every frame the pane publishes, which past the handover is the program answering. Frames the
    // program would have drawn anyway are counted too — that is why the line carries the count
    // rather than a rate, and why a quiet pane is the one to measure on.
    fun arrived() {
        if (!on || !open) return
        frames++
        span = started.elapsedNow().inWholeMilliseconds
        waitingSince?.let {
            waits += it.elapsedNow().inWholeMilliseconds
            waitingSince = null
        }
    }

    fun flush(keys: ScrollKeys) {
        if (!on || !open) return
        // Over the gesture's own span, first write to last frame: the idle gap that ends one is
        // not part of it, and counted in it would read as a stall.
        val ms = span
        val sorted = waits.sorted()
        emit(
            "KAMPR_SCROLL $platformLabel | keys=${keys.name}" +
                " | travel=$travel rows=$rows reports=$reports writes=$writes carried=$carried" +
                " frames=$frames ms=$ms" +
                " rate=${if (ms > 0) reports * 1000 / ms else 0}/s" +
                " fps=${if (ms > 0) frames * 1000 / ms else 0}" +
                " wait_p50=${sorted.getOrNull(sorted.size / 2) ?: -1}ms" +
                " wait_max=${sorted.lastOrNull() ?: -1}ms",
        )
        open = false
        reports = 0
        writes = 0
        carried = 0
        span = 0
        rows = 0
        travel = 0
        frames = 0
        waits.clear()
        waitingSince = null
    }
}
