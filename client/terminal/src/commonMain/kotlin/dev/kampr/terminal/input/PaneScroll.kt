package dev.kampr.terminal.input

import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
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
    // as a scroll, moving its view and nothing else. One report per notch, the way a terminal sends
    // one event per notch and lets the program choose its own step.
    Wheel(1),

    // Alternate scroll, and the default for everything else: the wheel becomes cursor keys. The
    // caret moves and the view follows it at the edge — which is exactly what herdr does with vim,
    // so the two surfaces behave alike rather than one of them inventing something.
    //
    // The **application** form, not `ESC [ A`. `less`, `man` and `vim` all set DECCKM, and the
    // normal form moved `less` by nothing at all where this form moves it a line a press (#390).
    CursorKeys(3),
    ;

    // How many go out for one notch of the wheel. A notch is three rows on this surface
    // (`WHEEL_ROWS`), and a cursor key is worth a row; a wheel report is worth whatever the program
    // says it is worth.
    val perNotch: Int
    constructor(perNotch: Int) {
        this.perNotch = perNotch
    }
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

// How often a queued report goes out, and it is a measurement rather than a taste (#527).
//
// A drag pushes **60 reports a second** into a pipeline that returns 20-35 distinct frames, and the
// excess is *merged rather than queued* — the same 26 reports bought 12 visible steps sent fast and
// 34 sent slow, so half the journey arrived as one jump instead of two. The round trip was never
// the limit: 14-31 ms throughout. A wheel never meets this because a hand makes 10-30 detents a
// second, which is the whole of "the desktop feels smooth and the phone does not".
private const val PACE_MS = 40L

// The most travel a finger may run ahead of the program, in reports.
//
// **Pacing without a bound is worse than not pacing.** The pipeline moves a program's view about
// 21 rows a second and a 250 ms swipe asks for 104, so one drag trails a few hundred milliseconds
// — which is momentum and reads as natural — and six in a row would queue *seconds* of scrolling
// the operator cannot cancel. Past this the oldest travel is dropped, which is the behaviour of a
// wheel that was turned faster than the program could follow.
private const val PENDING_CAP = 50

// The scroll a pane is given, by whichever gesture asked for it.
//
// A wheel hands over by notch and a finger by distance: once the surface underneath is spent, a drag
// asks for a row for every row it travels, which is the one-to-one a touch scroll is. The remainder
// is carried, or a slow drag rounds to nothing on every frame and the pane never moves at all.
//
// Positive is into history — the same sense `TerminalViewState.scrollY` uses — so a finger pulled
// *down* the screen asks for what is above it, and that is a scroll *up*.
//
// **The finger is paced and the wheel is not**, because only one of them outruns the program: a
// notch goes out the moment it is asked for, and a drag's rows are queued and released at
// [`PACE_MS`]. `scope` is what releases them; without one the queue only moves when [`drain`] is
// called, which is how a test drives it without a clock.
class PaneScroll(
    val keys: ScrollKeys,
    private val trace: ScrollTrace? = null,
    private val scope: CoroutineScope? = null,
    private val send: (String) -> Unit,
) {
    private var carried = 0f
    private var pending = 0
    private var atCol = 0
    private var atRow = 0
    private var pump: Job? = null

    val queued: Int get() = pending

    private fun report(up: Boolean, col: Int, row: Int) {
        trace?.sent(keys)
        send(scrollReport(keys, up, col, row))
    }

    fun notch(up: Boolean, col: Int, row: Int) {
        repeat(keys.perNotch) { report(up, col, row) }
    }

    fun refused(distance: Float, step: Float, col: Int, row: Int) {
        if (step <= 0f) return
        atCol = col
        atRow = row
        carried += distance
        while (carried >= step) {
            carried -= step
            pending++
        }
        while (carried <= -step) {
            carried += step
            pending--
        }
        pending = pending.coerceIn(-PENDING_CAP, PENDING_CAP)
        start()
    }

    // One report's worth of the queue, and whether any is left. Public because the pacing has to be
    // drivable without a clock.
    fun drain(): Boolean {
        if (pending == 0) return false
        val up = pending > 0
        pending += if (up) -1 else 1
        report(up, atCol, atRow)
        return pending != 0
    }

    private fun start() {
        val where = scope ?: return
        if (pump?.isActive == true) return
        pump = where.launch {
            // The first row of a gesture is not made to wait: the pace is a ceiling on the rate,
            // not a delay on the answer.
            while (drain()) delay(PACE_MS)
        }
    }

    // A gesture's leftovers are its own. Carried into the next one, the first row of a fresh
    // drag arrives before the finger has travelled it. What is already queued is *not* dropped —
    // a second swipe is more travel asked for, not a correction of the first.
    fun rest() {
        carried = 0f
        trace?.flush(keys)
    }
}
