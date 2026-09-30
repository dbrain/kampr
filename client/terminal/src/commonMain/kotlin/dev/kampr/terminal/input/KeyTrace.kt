package dev.kampr.terminal.input

import dev.kampr.terminal.bench.emitBench
import dev.kampr.terminal.bench.platformLabel
import kotlin.time.TimeMark
import kotlin.time.TimeSource

// Set at the entry point before anything composes, for the reason `scrollTracing` is.
var keyTracing: Boolean = false

// A burst of typing is over when nothing has been typed for this long, and a line is written for
// it then — or after this many keys, so a long stretch of typing reports while it is still going.
private const val BURST_GAP_MS = 1_000L
private const val BURST_KEYS = 50

// Past this a keystroke is felt as a halt rather than as latency.
private const val STALL_MS = 250L

// A key still unanswered this long moved nothing — an arrow against the end of a line, a press a
// dialog swallowed — and the next key is not charged for its wait.
private const val UNANSWERED_MS = 3_000L

// Where a keystroke's time went, measured where it is typed.
//
// The figure the app used to show was the node's ping to herdr, a leg of the trip every keystroke
// makes and nothing more: the program in the pane, herdr's frame, the wire back, and the client's
// own paint are all outside it, so a key that visibly halted behind "2.2 ms" was invisible to it.
// `echo` is the send to the first frame that moves the caret — the whole round trip, and not a
// spinner's repaint that happened to arrive first — and `paint` is that frame to the next one the
// client draws, which is the client's own share. A key sent before the last was answered waits
// on the same frame and is not timed twice.
//
// Every echo is handed to `echoed`, always; the log line is only written when tracing.
class KeyTrace(
    private val on: Boolean = keyTracing,
    private val emit: (String) -> Unit = ::emitBench,
    private val clock: TimeSource = TimeSource.Monotonic,
    var echoed: (Long) -> Unit = {},
) {
    private var lastAt: TimeMark = clock.markNow()
    private var waiting: TimeMark? = null
    private var painting: TimeMark? = null
    private var caretCol = -1
    private var caretRow = -1
    private var sentCol = -1
    private var sentRow = -1
    private var keys = 0
    private val echoes = mutableListOf<Long>()
    private val paints = mutableListOf<Long>()

    fun sent() {
        if (on && keys > 0 && (lastAt.elapsedNow().inWholeMilliseconds > BURST_GAP_MS || keys >= BURST_KEYS)) {
            flush()
        }
        lastAt = clock.markNow()
        keys++
        val stale = waiting?.let { it.elapsedNow().inWholeMilliseconds > UNANSWERED_MS } ?: true
        if (stale) {
            waiting = clock.markNow()
            sentCol = caretCol
            sentRow = caretRow
        }
    }

    fun frame(col: Int, row: Int) {
        caretCol = col
        caretRow = row
        val since = waiting ?: return
        if (col == sentCol && row == sentRow) return
        val ms = since.elapsedNow().inWholeMilliseconds
        waiting = null
        painting = clock.markNow()
        echoed(ms)
        if (on) echoes += ms
    }

    fun drawn() {
        val since = painting ?: return
        painting = null
        if (on) paints += since.elapsedNow().inWholeMilliseconds
    }

    fun flush() {
        if (!on || keys == 0) return
        val echo = echoes.sorted()
        val paint = paints.sorted()
        emit(
            "KAMPR_KEYS $platformLabel | keys=$keys" +
                " echo_p50=${echo.getOrNull(echo.size / 2) ?: -1}ms" +
                " echo_p95=${echo.getOrNull(echo.size * 95 / 100) ?: -1}ms" +
                " echo_max=${echo.lastOrNull() ?: -1}ms" +
                " paint_p50=${paint.getOrNull(paint.size / 2) ?: -1}ms" +
                " paint_max=${paint.lastOrNull() ?: -1}ms" +
                " stalls=${echo.count { it >= STALL_MS }}",
        )
        keys = 0
        echoes.clear()
        paints.clear()
    }
}
