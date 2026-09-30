package dev.kampr.terminal.input

import kotlin.math.abs
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

// What a pane is scrolled with when Kampr cannot scroll it itself.
//
// A harness that takes the alternate screen keeps no ring — herdr's is the main screen's, and there
// is none behind an alt screen (#387). So there is nothing above the viewport for Kampr to move
// into, and the gesture has to reach the program instead. That is not a Kampr invention: it is what
// every terminal does, herdr included, and what the operator already sees at the desk.
enum class ScrollKeys {
    // What a terminal sends when the program asked for the mouse: a scroll the program understands
    // as a scroll, moving its view and nothing else. How far a report moves Claude depends on what
    // arrived just before it (#567), which is `rowsFor`.
    Wheel,

    // Alternate scroll, and the default for everything else: the wheel becomes cursor keys. The
    // caret moves and the view follows it at the edge — which is exactly what herdr does with vim,
    // so the two surfaces behave alike rather than one of them inventing something.
    //
    // The **application** form, not `ESC [ A`. `less`, `man` and `vim` all set DECCKM, and the
    // normal form moved `less` by nothing at all where this form moves it a line a press (#390).
    CursorKeys,
}

// Harnesses measured to do better than the default: they take a real wheel report, so their view
// moves without their caret moving. Claude Code 2.1.252 sets `?1000h` and `?1006h` at startup and
// scrolls its transcript on one (#388). Same stance `SUBMIT_KEYS` takes on the node — a harness
// nobody has probed is not guessed at, it just gets the default.
private val TAKES_THE_WHEEL = setOf("claude")

// Which keys this pane is scrolled with, or nothing at all.
//
// **`cmd` is the gate, and it fails closed.** It is the pane's foreground job as the node reports
// it, and it is null both when the pane is sitting at its prompt and when nothing could tell —
// ble.sh keeps a job in the shell's own process group, so herdr answers nothing for it (#297) and
// the node's procfs walk is what recovers the name. Either way, null means the shell may be the
// thing listening, and cursor keys into a shell's line editor recall its history. Nothing is sent
// there, whatever else is known about the pane: a harness label outlives the harness, so `agent`
// alone would still be typing into the prompt a minute after the agent quit.
fun paneScrollKeys(agent: String?, cmd: String?): ScrollKeys? = when {
    cmd == null -> null
    agent != null && agent in TAKES_THE_WHEEL -> ScrollKeys.Wheel
    else -> ScrollKeys.CursorKeys
}

// SGR (`?1006h`): a wheel is a press with no release — 64 up, 65 down — at 1-based cell coordinates.
internal fun scrollReport(keys: ScrollKeys, up: Boolean, col: Int, row: Int): String = when (keys) {
    ScrollKeys.Wheel -> "\u001b[<${if (up) 64 else 65};${col + 1};${row + 1}M"
    ScrollKeys.CursorKeys -> if (up) "\u001bOA" else "\u001bOB"
}

// What `count` reports sent in one write move the program, and it is Claude's own arithmetic, read
// out of 2.1.285 and measured to the row for 1 to 40 reports (#567). A report landing within
// 40 ms of the last one ramps: +0.3 rows each, floored, from 1 to a ceiling of 6. And the first
// report after a change of direction moves nothing at all. `CursorKeys` programs do neither.
//
// This is what made the finger tick. A pump releasing one report every 40 ms sits exactly on the
// reset, so every report moved one row — ~21 rows a second however far the finger went.
internal fun ScrollKeys.rowsFor(count: Int, reversed: Boolean): Int = when (this) {
    ScrollKeys.CursorKeys -> count
    ScrollKeys.Wheel -> {
        var rows = 0
        // Summed in doubles as Claude's JavaScript sums them: the eleventh step is 3.999… and
        // floors to 3, which is why 15 reports measured 39 rows and not 40.
        var mult = 1.0
        repeat((if (reversed) count - 1 else count).coerceAtLeast(0)) {
            rows += mult.toInt()
            mult = (mult + 0.3).coerceAtMost(6.0)
        }
        rows
    }
}

// The biggest write that does not overshoot. Travel it cannot land exactly waits for the next one.
private fun ScrollKeys.reportsFor(rows: Int, reversed: Boolean): Int {
    var count = 0
    while (count < MAX_REPORTS && rowsFor(count + 1, reversed) <= rows) count++
    return count
}

// Measured faithful up to here, and 187 rows of Claude — more than a finger travels between two
// frames.
private const val MAX_REPORTS = 40

// **The ramp only resets after 40 ms of quiet at Claude's end**, so the next write goes out that
// long after the frame that answered the last one — which cannot have been drawn before the last
// one landed. Sent sooner, it lands inside the ramp and moves further than the model says (#567:
// two writes of 5 moved 19 rows 35 ms apart and 12 rows 45 ms apart).
private const val QUIET_MS = 45L

// A write that changed nothing on screen — the top of the transcript — is answered by no frame.
private const val UNANSWERED_MS = 150L

// The scroll a pane is given, by whichever gesture asked for it.
//
// Both hand over by distance: once the surface underneath is spent, a wheel or a drag asks for a
// row for every row it travels. A drag's remainder is carried here, or a slow drag rounds to
// nothing on every frame and the pane never moves at all; a wheel arrives in whole rows already.
//
// Positive is into history — the same sense `TerminalViewState.scrollY` uses — so a finger pulled
// *down* the screen asks for what is above it, and that is a scroll *up*.
//
// **A drag goes out as whole writes and the wheel does not.** Each write carries every row the
// finger travelled since the last, turned into however many reports land that many rows, so the
// program's view keeps up with the finger and nothing is left to trickle out once it lifts. `scope`
// is what releases them; without one the rows only move when [`drain`] is called, which is how a
// test drives it without a clock.
class PaneScroll(
    val keys: ScrollKeys,
    private val trace: ScrollTrace? = null,
    private val scope: CoroutineScope? = null,
    private val send: (String) -> Unit,
) {
    private var carried = 0f
    private var pending = 0
    private var lastUp: Boolean? = null
    private var atCol = 0
    private var atRow = 0
    private var pump: Job? = null
    private val answers = Channel<Boolean>(Channel.CONFLATED)

    val queued: Int get() = pending

    private val cap = keys.rowsFor(MAX_REPORTS, reversed = false)

    // Whole rows, because the fractions a trackpad hands over are carried by the wheel until they
    // make one: a report for every tiny delta ran Claude's view a row per event. Unpaced: a hand
    // makes 10-30 detents a second, and the ramp is what Claude does for a wheel spun fast.
    fun wheel(rows: Int, col: Int, row: Int) {
        if (rows == 0) return
        lastUp = rows > 0
        repeat(abs(rows)) {
            trace?.sent(keys, reports = 1, rows = 1)
            send(scrollReport(keys, rows > 0, col, row))
        }
    }

    fun refused(distance: Float, step: Float, col: Int, row: Int) {
        if (step <= 0f) return
        atCol = col
        atRow = row
        carried += distance
        val rows = (carried / step).toInt()
        carried -= rows * step
        trace?.travelled(rows)
        pending = (pending + rows).coerceIn(-cap, cap)
        start()
    }

    // A frame came back from the pane.
    fun answered() {
        answers.trySend(true)
    }

    // Everything travelled so far as one write, and whether any is left over. Public because the
    // release has to be drivable without a clock.
    fun drain(): Boolean {
        if (pending == 0) return false
        val up = pending > 0
        val reversed = lastUp != null && lastUp != up
        val count = keys.reportsFor(abs(pending), reversed)
        val rows = keys.rowsFor(count, reversed)
        pending -= if (up) rows else -rows
        lastUp = up
        trace?.sent(keys, reports = count, rows = rows)
        send(scrollReport(keys, up, atCol, atRow).repeat(count))
        return pending != 0
    }

    private fun start() {
        val where = scope ?: return
        if (pump?.isActive == true) return
        pump = where.launch {
            // The first write of a gesture is not made to wait: the gap is between writes, not a
            // delay on the answer.
            while (pending != 0) {
                answers.tryReceive()
                drain()
                // A timer racing the frame rather than `withTimeoutOrNull`, whose clock is not the
                // dispatcher's on every platform: under the Compose test clock it waited in real time.
                val unanswered = launch {
                    delay(UNANSWERED_MS)
                    answers.trySend(false)
                }
                if (answers.receive()) delay(QUIET_MS)
                unanswered.cancel()
            }
        }
    }

    // A gesture's leftover fraction of a row is its own. Carried into the next one, the first row
    // of a fresh drag arrives before the finger has travelled it.
    fun rest() {
        carried = 0f
        trace?.flush(keys)
    }
}
