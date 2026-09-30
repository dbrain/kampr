package dev.kampr.terminal

import dev.kampr.shared.wire.Cursor
import dev.kampr.terminal.input.LocalEcho
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.time.Duration.Companion.milliseconds
import kotlin.time.TimeMark
import kotlin.time.TimeSource

// Drawing a typed character before the pane echoes it, the way mosh does, and taking it back the
// moment the pane disagrees. A guess is only ever made on a line that has been echoing exactly what
// was typed, where it was typed — so a password prompt, a dialog that takes a digit, or a program
// that draws its own keys shows at most the one character it took to find out.
class LocalEchoTest {
    private class Clock : TimeSource {
        var now = 0L
        override fun markNow(): TimeMark = object : TimeMark {
            val at = now
            override fun elapsedNow() = (now - at).milliseconds
        }
    }

    // A one-line pane of 20 columns: a prompt, and whatever the program has echoed after it.
    private class Line(val cols: Int = 20) {
        val clock = Clock()
        val echo = LocalEcho(clock)
        val cells = IntArray(cols) { ' '.code }
        var caret = Cursor(2, 0, true)

        init {
            cells[0] = '$'.code
        }

        fun type(text: String) = echo.typed(text, caret, cols)

        fun echoes(text: String) {
            for (ch in text) {
                cells[caret.col] = ch.code
                caret = Cursor(caret.col + 1, caret.row, caret.visible)
            }
            frame()
        }

        fun frame() = echo.frame({ col, row -> if (row == 0) cells[col] else ' '.code }, caret)

        fun shown(): String = (0 until echo.shown).joinToString("") { echo.glyph(it).toChar().toString() }
    }

    @Test
    fun nothingIsGuessedUntilTheLineHasEchoedWhatWasTyped() {
        val line = Line()
        line.type("l")
        assertEquals("", line.shown(), "a line nobody has watched echo gets no guess")
        line.echoes("l")
        line.type("s")
        assertEquals("", line.shown(), "one echo is not yet a line that echoes")
        line.echoes("s")
        line.type(" -")
        assertEquals(" -", line.shown())
        assertEquals(Cursor(6, 0, true), line.echo.caret(line.caret), "the caret is drawn after what was guessed")
    }

    @Test
    fun theEchoThatAgreesTakesTheGuessAwaySoNothingIsDrawnTwice() {
        val line = Line()
        line.type("ab")
        line.echoes("ab")
        line.type("cd")
        assertEquals("cd", line.shown())
        line.echoes("c")
        assertEquals("d", line.shown())
        assertEquals(Cursor(6, 0, true), line.echo.caret(line.caret), "the caret stays after the guess still standing")
        line.echoes("d")
        assertEquals("", line.shown())
        assertEquals(line.caret, line.echo.caret(line.caret))
    }

    // `sudo` asking for a password after a line that was echoing: the guess is drawn for one round
    // trip and gone the moment the pane answers without it — and the line is not trusted again
    // until it echoes again.
    @Test
    fun aKeyThePaneDidNotEchoTakesEveryGuessBackAndStopsGuessing() {
        val line = Line()
        line.type("ab")
        line.echoes("ab")
        line.type("x")
        assertEquals("x", line.shown())
        line.caret = Cursor(line.caret.col + 1, 0, true)
        line.frame()
        assertEquals("", line.shown(), "the pane moved past the guess and drew something else")
        line.type("y")
        assertEquals("", line.shown(), "a line that just disagreed is not guessed on")
    }

    @Test
    fun aGuessNothingEverAnsweredIsTakenBack() {
        val line = Line()
        line.type("ab")
        line.echoes("ab")
        line.type("x")
        line.clock.now += 400L
        line.echo.expire()
        assertEquals("x", line.shown(), "a slow echo is still an echo")
        line.clock.now += 2_000L
        line.echo.expire()
        assertEquals("", line.shown())
        line.type("y")
        assertEquals("", line.shown())
    }

    // Enter runs something, and what runs next may not echo; so does anything that is not a
    // character — an arrow, a backspace, a control key — which moves the caret somewhere a guess
    // cannot follow.
    @Test
    fun anythingButAPrintableCharacterEndsTheTrust() {
        for (key in listOf("\r", "\u007f", "\u001b[A", "\u0003")) {
            val line = Line()
            line.type("ab")
            line.echoes("ab")
            line.type(key)
            line.type("c")
            assertEquals("", line.shown(), "${key.map { it.code }} left the line trusted")
        }
    }

    // A dialog opening moves the caret somewhere the operator did not type to, and a digit into it
    // answers the question rather than being drawn.
    @Test
    fun aCaretThatMovesByItselfEndsTheTrust() {
        val line = Line()
        line.type("ab")
        line.echoes("ab")
        line.caret = Cursor(0, 0, true)
        line.frame()
        line.type("1")
        assertEquals("", line.shown())
    }

    @Test
    fun aHiddenCaretOrTheEndOfTheLineIsNeverGuessedAt() {
        val hidden = Line()
        hidden.type("ab")
        hidden.echoes("ab")
        hidden.caret = hidden.caret.copy(visible = false)
        hidden.type("c")
        assertEquals("", hidden.shown(), "a full-screen program hides its caret before it draws")

        val edge = Line(cols = 6)
        edge.type("ab")
        edge.echoes("ab")
        edge.type("cd")
        assertEquals("c", edge.shown(), "a guess never goes into the last column, where the line wraps")
    }

    // Claude's composer, as measured: `❯ ` then the text from column 2, and the key that would land
    // two columns short of the edge starts a new line at column 2 instead. The box grows upward, so
    // the line typed so far scrolls up a row and the caret stays on the same screen row, back at
    // the left. One frame per key, as it was measured at a typing pace.
    private class Composer(val cols: Int = 12) {
        val clock = Clock()
        val echo = LocalEcho(clock)
        val rows = Array(3) { IntArray(cols) { ' '.code } }
        var caret = Cursor(2, 2, true)

        init {
            rows[2][0] = '❯'.code
        }

        fun type(text: String) = echo.typed(text, caret, cols)

        fun echoes(text: String) {
            for (ch in text) {
                if (caret.col == cols - 2) {
                    rows[0] = rows[1]
                    rows[1] = rows[2]
                    rows[2] = IntArray(cols) { ' '.code }
                    caret = Cursor(2, caret.row, caret.visible)
                }
                rows[caret.row][caret.col] = ch.code
                caret = Cursor(caret.col + 1, caret.row, caret.visible)
                echo.frame({ col, row -> rows[row][col] }, caret)
            }
        }

        fun shown(): String = (0 until echo.shown).joinToString("") { echo.glyph(it).toChar().toString() }
    }

    @Test
    fun aComposerThatWrapsShortOfTheEdgeTakesTheGuessBackOnTheFrameThatWrapped() {
        val box = Composer()
        box.type("ab")
        box.echoes("ab")
        box.type("cdefghi")
        box.echoes("cdefgh")
        assertEquals("i", box.shown(), "the key that wraps is guessed where the line would have gone")
        box.echoes("i")
        assertEquals("", box.shown(), "the composer put it on a new line and the guess stayed at the edge")
        assertEquals(box.caret, box.echo.caret(box.caret), "the caret was drawn at the edge while the real one moved on")
    }

    @Test
    fun aLineThatWrappedIsNotGuessedAtItsWrapColumnAgain() {
        val box = Composer()
        box.type("ab")
        box.echoes("ab")
        box.type("cdefghi")
        box.echoes("cdefghi")
        box.type("jk")
        box.echoes("jk")
        box.type("lmnopq")
        assertEquals("lmnop", box.shown(), "the column the composer wrapped at is guessed at again")

        val wider = Composer(cols = 14)
        wider.type("ab")
        wider.echoes("ab")
        wider.type("cdefghi")
        wider.echoes("cdefghi")
        wider.type("jk")
        assertEquals("jk", wider.shown(), "a wrap nothing has shown at this width is still guessed up to the edge")
    }

    // A program that clears its line and redraws it from the left has moved the caret back without
    // wrapping anything; that disagrees with the guess, and teaches nothing about where lines end.
    @Test
    fun aCaretSentBackToTheLeftTakesTheGuessBackWithoutInventingAWrap() {
        val line = Line()
        line.type("ab")
        line.echoes("ab")
        line.type("x")
        line.caret = Cursor(2, 0, true)
        line.frame()
        assertEquals("", line.shown(), "the pane sent the caret back and the guess stayed")
        line.type("cd")
        line.echoes("cd")
        line.type("efg")
        assertEquals("efg", line.shown(), "a redraw was taken for a wrap")
    }
}
