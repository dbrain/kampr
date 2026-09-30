package dev.kampr.terminal.input

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.wire.Cursor
import kotlin.time.TimeMark
import kotlin.time.TimeSource

// Confirmed echoes a line needs before a keystroke on it is drawn ahead of its answer.
private const val TRUST = 2

// A guess nothing has answered for this long is taken back, and the line is not trusted again
// until it echoes again.
private const val UNANSWERED_MS = 1_500L

private const val MAX_PENDING = 64

// Typed characters drawn before the pane echoes them, and taken back the moment it disagrees.
//
// Every printable key is written down where it should land; the frame that puts it there confirms
// it, and a frame that moves past without it, or a wait with no answer, takes back every guess and
// the line's trust with it. Only a line that has echoed its last keys exactly where they were typed
// is guessed on, and anything that is not a character — Enter, an arrow, a backspace, a control
// key — or a caret that moved when nothing typed moved it, ends that. So a password prompt, a
// dialog that takes a digit, or a program that draws keys somewhere of its own costs one guessed
// character for one round trip, and a shell line or an agent's composer draws the key at once.
//
// Held in arrays rather than objects because the renderer reads it on every frame (#58-#62).
class LocalEcho(private val clock: TimeSource = TimeSource.Monotonic) {
    private val rows = IntArray(MAX_PENDING)
    private val cols = IntArray(MAX_PENDING)
    private val glyphs = IntArray(MAX_PENDING)
    private var pending = 0
    private var firstShown = MAX_PENDING
    private var trusted = 0
    private var since: TimeMark? = null
    private var expectCol = -1
    private var expectRow = -1

    // Snapshot state: the surface redraws when a guess appears or is taken back.
    var shown by mutableIntStateOf(0)
        private set

    fun glyph(i: Int): Int = glyphs[firstShown + i]

    val row: Int get() = rows[firstShown]

    fun col(i: Int): Int = cols[firstShown + i]

    fun caret(actual: Cursor): Cursor =
        if (shown == 0) actual else Cursor(cols[pending - 1] + 1, rows[pending - 1], actual.visible)

    fun typed(text: String, caret: Cursor, width: Int) {
        if (text.isEmpty()) return
        if (!caret.visible || text.any { it.code !in 0x20..0x7e }) {
            distrust()
            return
        }
        for (ch in text) {
            val row = if (pending == 0) caret.row else rows[pending - 1]
            val col = if (pending == 0) caret.col else cols[pending - 1] + 1
            // The last column is where the line wraps, and where it wraps to is the program's
            // business. The key goes unwritten, so its echo reads as a caret nothing explained.
            if (col >= width - 1 || pending == MAX_PENDING) break
            if (pending == 0) since = clock.markNow()
            rows[pending] = row
            cols[pending] = col
            glyphs[pending] = ch.code
            if (trusted >= TRUST && firstShown == MAX_PENDING) firstShown = pending
            pending++
        }
        publish()
    }

    fun frame(cell: (col: Int, row: Int) -> Int, caret: Cursor) {
        if (pending == 0) {
            if (expectCol >= 0 && (caret.col != expectCol || caret.row != expectRow)) trusted = 0
            expectCol = caret.col
            expectRow = caret.row
            return
        }
        var confirmed = 0
        while (confirmed < pending) {
            val row = rows[confirmed]
            val col = cols[confirmed]
            if (cell(col, row) == glyphs[confirmed]) {
                confirmed++
                continue
            }
            if (caret.row != row || caret.col > col) {
                distrust()
                return
            }
            break
        }
        if (confirmed == 0) return
        trusted += confirmed
        expectCol = cols[confirmed - 1] + 1
        expectRow = rows[confirmed - 1]
        drop(confirmed)
    }

    fun expire() {
        val at = since ?: return
        if (pending > 0 && at.elapsedNow().inWholeMilliseconds > UNANSWERED_MS) distrust()
    }

    // How long until [expire] could take something back, for a caller that has to wake up for it.
    val deadlineMs: Long get() = UNANSWERED_MS + 1

    private fun drop(count: Int) {
        rows.copyInto(rows, 0, count, pending)
        cols.copyInto(cols, 0, count, pending)
        glyphs.copyInto(glyphs, 0, count, pending)
        pending -= count
        firstShown = if (firstShown == MAX_PENDING) MAX_PENDING else (firstShown - count).coerceAtLeast(0)
        if (pending == 0) {
            firstShown = MAX_PENDING
            since = null
        } else {
            since = clock.markNow()
        }
        publish()
    }

    private fun distrust() {
        pending = 0
        firstShown = MAX_PENDING
        trusted = 0
        since = null
        expectCol = -1
        expectRow = -1
        publish()
    }

    private fun publish() {
        shown = if (firstShown == MAX_PENDING) 0 else pending - firstShown
    }
}
