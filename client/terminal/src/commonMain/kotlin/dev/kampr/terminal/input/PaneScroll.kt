package dev.kampr.terminal.input

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

// The scroll a pane is given, by whichever gesture asked for it.
//
// A wheel hands over by notch and a finger by distance: once the surface underneath is spent, a drag
// asks for a row for every row it travels, which is the one-to-one a touch scroll is. The remainder
// is carried, or a slow drag rounds to nothing on every frame and the pane never moves at all.
//
// Positive is into history — the same sense `TerminalViewState.scrollY` uses — so a finger pulled
// *down* the screen asks for what is above it, and that is a scroll *up*.
class PaneScroll(val keys: ScrollKeys, private val send: (String) -> Unit) {
    private var carried = 0f

    fun notch(up: Boolean, col: Int, row: Int) {
        repeat(keys.perNotch) { send(scrollReport(keys, up, col, row)) }
    }

    fun refused(distance: Float, step: Float, col: Int, row: Int) {
        if (step <= 0f) return
        carried += distance
        while (carried >= step) {
            carried -= step
            send(scrollReport(keys, up = true, col = col, row = row))
        }
        while (carried <= -step) {
            carried += step
            send(scrollReport(keys, up = false, col = col, row = row))
        }
    }

    // A gesture's leftovers are its own. Carried into the next one, the first row of a fresh drag
    // arrives before the finger has travelled it.
    fun rest() {
        carried = 0f
    }
}
