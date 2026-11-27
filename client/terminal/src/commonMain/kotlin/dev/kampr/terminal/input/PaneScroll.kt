package dev.kampr.terminal.input

import dev.kampr.shared.wire.PaneModes
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlin.math.abs
import kotlin.time.TimeMark
import kotlin.time.TimeSource

// What a pane is scrolled with when Kampr cannot scroll it itself.
//
// A harness that takes the alternate screen keeps no ring — herdr's is the main screen's, and there
// is none behind an alt screen (#387). So there is nothing above the viewport for Kampr to move
// into, and the gesture has to reach the program instead. That is not a Kampr invention: it is what
// every terminal does, herdr included, and what the operator already sees at the desk.
enum class ScrollKeys {
    // What a terminal sends when the program asked for the mouse: a scroll the program understands
    // as a scroll, moving its view and nothing else. How far a report moves Claude depends on what
    // arrived just before it (#567), which is `land`.
    Wheel,

    // A program that took the mouse and moves a fixed three rows a report: muffit's TUI enables
    // mouse capture and scrolls its transcript `WHEEL_LINES` (3) a wheel report, with no ramp and no
    // loss on a change of direction (`muffit-tui/src/keys.rs`). Cursor keys went to its composer and
    // recalled the operator's input history.
    Notch,

    // A program that took the mouse and is not one of the two above: a report moves it a row, which
    // scrolls it correctly if not smoothly, so a program nobody has measured degrades in smoothness
    // rather than breaking.
    Report,

    // Alternate scroll, and the default for everything else: the wheel becomes cursor keys. The
    // caret moves and the view follows it at the edge — which is exactly what herdr does with vim,
    // so the two surfaces behave alike rather than one of them inventing something.
    //
    // The **application** form, not `ESC [ A`. `less`, `man` and `vim` all set DECCKM, and the
    // normal form moved `less` by nothing at all where this form moves it a line a press (#390).
    CursorKeys,
}

// How far a report moves a program that took the mouse, by name, because that is what is measured
// and nothing the program says. Claude ramps (#567); muffit's TUI moves a fixed three rows a report.
// A program in neither gets a row a report, which scrolls it correctly if not smoothly, so a program
// nobody has measured degrades in smoothness rather than breaking.
private val RAMPS = setOf("claude")

private val FIXED_NOTCH = setOf("muffit")

// herdr labels a muffit TUI pane nothing and names its job `muffit-tui` however it was started
// (#699), so the job names it. Claude is still named by its label alone: a ring under an unlabelled
// `claude` job is history to scroll until its label arrives.
private val BY_JOB = mapOf("muffit-tui" to "muffit")

private fun program(agent: String?, cmd: String?): String? = agent ?: BY_JOB[cmd?.substringAfterLast('/')]

private fun tuned(program: String?): ScrollKeys? = when (program) {
    in RAMPS -> ScrollKeys.Wheel
    in FIXED_NOTCH -> ScrollKeys.Notch
    else -> null
}

private const val NOTCH_ROWS = 3

// Which keys this pane is scrolled with, or nothing at all.
//
// **What the program asked for decides, when the node said** (`modes`, docs/26): a mouse mode it set
// is a report, the alternate screen with alternate scroll on is cursor keys, and anything else is
// nothing, whatever the pane's `agent` says. A shell at its prompt has set none of those. A mouse
// mode without SGR encoding is sent nothing: its reports stop at column 223 and no program measured
// needs them.
//
// **Without modes, `cmd` is the gate, and it fails closed**, for a herdr pane (#292, #686) and a node
// that predates the field. It is the pane's foreground job as the node reports it, and it is null both
// when the pane is sitting at its prompt and when nothing could tell — ble.sh keeps a job in the
// shell's own process group, so herdr answers nothing for it (#297) and the node's procfs walk is what
// recovers the name. Either way, null means the shell may be the thing listening, and cursor keys into
// a shell's line editor recall its history. Nothing is sent there, whatever else is known about the
// pane: a harness label outlives the harness, so `agent` alone would still be typing into the prompt a
// minute after the agent quit.
fun paneScrollKeys(agent: String?, cmd: String?, modes: PaneModes? = null): ScrollKeys? = when {
    modes != null -> when {
        modes.tracksMouse -> if (modes.mouseSgr) tuned(program(agent, cmd)) ?: ScrollKeys.Report else null
        modes.altScreen && modes.altScroll -> ScrollKeys.CursorKeys
        else -> null
    }

    cmd == null -> null

    else -> tuned(program(agent, cmd)) ?: ScrollKeys.CursorKeys
}

// Whether the wheel goes to the program while Kampr holds a ring for the pane. The ring under a
// program that holds the alternate screen is the shell session from before it started. A native
// pane's node drops it under any such program, but a herdr pane's node only under one it can name
// (#686, #699), and an older node never named muffit, so muffit keeps a by-name fallback here.
//
// **With modes, rules 1 and 2 of docs/26 beat the ring**, as in every terminal. **Without them, only a
// program measured to hold the alternate screen for its whole run does**: muffit-tui sets `?1049h`
// before it draws anything, where Claude's classic renderer writes its turns into a ring that is
// the conversation itself (#588), so a ring under a labelled Claude is history to scroll.
private val HOLDS_THE_SCREEN = setOf("muffit")

fun paneWheelBeatsTheRing(agent: String?, cmd: String?, modes: PaneModes?): Boolean = modes != null || program(agent, cmd) in HOLDS_THE_SCREEN

// SGR (`?1006h`): a wheel is a press with no release — 64 up, 65 down — at 1-based cell coordinates.
internal fun scrollReport(keys: ScrollKeys, up: Boolean, col: Int, row: Int): String = when (keys) {
    ScrollKeys.Wheel, ScrollKeys.Notch, ScrollKeys.Report -> "\u001b[<${if (up) 64 else 65};${col + 1};${row + 1}M"
    ScrollKeys.CursorKeys -> if (up) "\u001bOA" else "\u001bOB"
}

// What `count` reports sent in one write move the program, and where they leave its ramp. It is
// Claude's own arithmetic, read out of 2.1.285 and measured to the row for 1 to 40 reports (#567):
// a report landing within 40 ms of the last one ramps, +0.3 rows each, floored, from 1 to a ceiling
// of 6, and the first report after a change of direction moves nothing at all. `ramp` is where the
// last write left the multiplier when this one lands inside that window — the ramp is Claude's,
// not the write's, and carries across writes exactly (#569) — and null starts it again.
// `CursorKeys` programs do none of it.
internal class Landing(val rows: Int, val ramp: Double?)

internal fun ScrollKeys.land(count: Int, reversed: Boolean, ramp: Double? = null): Landing = when (this) {
    ScrollKeys.CursorKeys, ScrollKeys.Report -> Landing(count, null)

    ScrollKeys.Notch -> Landing(count * NOTCH_ROWS, null)

    ScrollKeys.Wheel -> {
        var rows = 0
        // Summed in doubles as Claude's JavaScript sums them: the eleventh step is 3.999… and
        // floors to 3, which is why 15 reports measured 39 rows and not 40.
        var mult = if (reversed) null else ramp
        repeat((if (reversed) count - 1 else count).coerceAtLeast(0)) {
            val next = mult?.let { (it + 0.3).coerceAtMost(6.0) } ?: 1.0
            rows += next.toInt()
            mult = next
        }
        Landing(rows, mult)
    }
}

// The biggest write that does not overshoot. Travel it cannot land exactly waits for the next one,
// and a carried ramp can make even one report too many.
private fun ScrollKeys.reportsFor(rows: Int, reversed: Boolean, ramp: Double?): Int {
    var count = 0
    while (count < MAX_REPORTS && land(count + 1, reversed, ramp).rows <= rows) count++
    return count
}

// Measured faithful up to here, and 187 rows of Claude — more than a finger travels between two
// frames.
private const val MAX_REPORTS = 40

// **A write goes out on one side of Claude's 40 ms reset or well clear of the other, never near
// it**, because the gap Claude sees is the gap sent plus whatever the link does to it. Inside:
// every tick, which is Claude's own ~17 ms repaint (#570), sized from where the last write
// left the ramp. A tick the dispatcher ran late past `CARRY_MS` is not trusted to carry. That
// leaves 16 ms of link jitter before a carried write lands outside the window and moves less than
// it was sized for (#571: exact at 0-20 ms, one row over at 0-30). What it cannot absorb is
// herdr holding one write for 100 ms (#445), which resets the ramp under a run the model thinks is
// carried: 61 rows for 100 (#572). That is ~1 write in 1000 at this cadence.
private const val TICK_MS = 16L
private const val CARRY_MS = 24L

// Outside: a fresh write waits for the frame that came back after the last one and then long
// enough that Claude's window has shut. The tick that found nothing to carry comes first, so that
// is never sooner than 61 ms after the last write — 21 ms of margin for a link that bunches the two
// up. Sent sooner, it lands inside the ramp and moves further than the model says (#567: two
// writes of 5 moved 19 rows 35 ms apart and 12 rows 45 ms apart).
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
// program's view keeps up with the finger and nothing is left to trickle out once it lifts. While
// the finger keeps moving a write goes out every tick with Claude's ramp carried across them; when
// it slows past what one carried report is worth, the ramp is let reset before the next. `scope`
// is what releases them; without one the rows only move when [`drain`] is called, which is how a
// test drives it without a clock.
class PaneScroll(
    val keys: ScrollKeys,
    private val trace: ScrollTrace? = null,
    private val scope: CoroutineScope? = null,
    private val clock: TimeSource = TimeSource.Monotonic,
    private val send: (String) -> Unit,
) {
    private var carried = 0f
    private var pending = 0
    private var lastUp: Boolean? = null
    private var ramp: Double? = null
    private var wrote: TimeMark = clock.markNow()
    private var atCol = 0
    private var atRow = 0
    private var pump: Job? = null
    private var overshot = 0
    private val answers = Channel<Boolean>(Channel.CONFLATED)

    val queued: Int get() = pending

    private val cap = keys.land(MAX_REPORTS, reversed = false).rows

    // Whole rows, because the fractions a trackpad hands over are carried by the wheel until they
    // make one: a report for every tiny delta ran Claude's view a row per event. Unpaced: a hand
    // makes 10-30 detents a second, and the ramp is what Claude does for a wheel spun fast.
    fun wheel(rows: Int, col: Int, row: Int) {
        if (rows == 0) return
        if (lastUp != (rows > 0)) overshot = 0
        lastUp = rows > 0
        val reports = if (keys == ScrollKeys.Notch) notches(abs(rows)) else abs(rows)
        repeat(reports) {
            trace?.sent(keys, reports = 1, rows = 1)
            send(scrollReport(keys, rows > 0, col, row))
        }
    }

    // A part-notch is rounded up so that it moves the program at all, and the rows that moved
    // beyond it are paid back by the next ones, or a trackpad's row-an-event stream moves muffit
    // three rows for each.
    private fun notches(rows: Int): Int {
        val owed = rows - overshot
        val reports = if (owed <= 0) 0 else (owed + NOTCH_ROWS - 1) / NOTCH_ROWS
        overshot = reports * NOTCH_ROWS - owed
        return reports
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

    // Everything travelled so far that one write can land, and whether any is left over. `carry`
    // sizes it from where the last write left Claude's ramp, which is only true of a write that
    // lands inside the window. Public because the release has to be drivable without a clock.
    fun drain(carry: Boolean = false): Boolean {
        write(carry)
        return pending != 0
    }

    private fun write(carry: Boolean): Boolean {
        if (pending == 0) return false
        val up = pending > 0
        val reversed = lastUp != null && lastUp != up
        val from = if (carry) ramp else null
        val count = keys.reportsFor(abs(pending), reversed, from)
        if (count == 0) return false
        val landing = keys.land(count, reversed, from)
        pending -= if (up) landing.rows else -landing.rows
        lastUp = up
        ramp = landing.ramp
        answers.tryReceive()
        wrote = clock.markNow()
        trace?.sent(keys, reports = count, rows = landing.rows, carried = carry)
        send(scrollReport(keys, up, atCol, atRow).repeat(count))
        return true
    }

    private fun start() {
        val where = scope ?: return
        if (pump?.isActive == true) return
        pump = where.launch {
            // The first write of a gesture is not made to wait: the gap is between writes, not a
            // delay on the answer.
            while (pending != 0) {
                write(carry = false)
                do {
                    delay(TICK_MS)
                } while (wrote.elapsedNow().inWholeMilliseconds <= CARRY_MS && write(carry = true))
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
