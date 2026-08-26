package dev.kampr.conversation

import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertNotNull

private const val AT = "2026-08-23T09:00:00.000Z"
private val NOW = requireNotNull(parseIsoMillis(AT))

class TurnFoldTest {
    // A header is a row of chrome and a 36 dp target, so it has to buy more than it costs. Nothing
    // here would save a line by folding: a reply is short by construction and already says who
    // wrote it by where it sits, a turn of nothing but calls is the run's business and wears a
    // chevron already, and a preview is being rewritten under the reader as they look at it.
    @Test
    fun nothingThatWouldNotSaveALineWearsAHeader() {
        val cases = mapOf(
            "a reply" to proseTurn("u-1", "run the tests\nand the lint\nplease", role = "user"),
            "one line" to proseTurn("a-1", "Done."),
            "one paragraph that merely wraps" to proseTurn("a-2", "The letterbox came from min where max was meant, and every pane on a phone showed it."),
            "a turn of calls" to bashTurn("a-3", "cargo test", "done", 3),
            "a preview" to proseTurn(LIVE_TURN_ID, "I will write the file,\nthen explain\nwhat it does."),
        )
        for ((what, turn) in cases) assertNull(foldKey(turn), "$what was given a header")
    }

    @Test
    fun ananswerLongEnoughToBeWorthTheControlGetsOne() {
        assertEquals("fold:a-9", foldKey(proseTurn("a-9", "First.\n\nSecond.\n\nThird.")))
        assertNotNull(
            foldKey(Turn("a-10", "assistant", AT, listOf(Block.Md("here:"), Block.Code("bash", "ls")))),
            "prose and a fence are two pieces and fold together",
        )
    }

    // The node copies whatever the harness wrote, and the harness is not this client's to trust:
    // an age is correct in every timezone, which the time of day in an unread offset is not.
    @Test
    fun theStampIsAnAgeAndAnUnreadableOneIsNoStampAtAll() {
        assertEquals("now", turnStamp(AT, NOW))
        assertEquals("12m", turnStamp(AT, NOW + 12 * 60_000))
        assertEquals("3h", turnStamp(AT, NOW + 3 * 3_600_000))
        assertEquals("2d", turnStamp(AT, NOW + 2 * 86_400_000))
        assertNull(turnStamp(null, NOW))
        assertNull(turnStamp("whenever", NOW))
    }

    // What a folded turn says about itself, and it is a line of prose rather than a line of
    // markdown: a row reading "## Corrections and event" is the syntax, not the message.
    @Test
    fun theGistIsTheFirstLineOfTheMessageWithoutItsMarkdown() {
        assertEquals("Corrections and event behaviour", turnGist(proseTurn("a", "## Corrections and event behaviour\n\nFive rows.")))
        assertEquals("Herdr caps pane.read at 1000 lines", turnGist(proseTurn("a", "> Herdr caps `pane.read` at 1000 lines\n> and there is no offset.")))
        assertEquals("the first thing it says", turnGist(proseTurn("a", "\n\n  the first thing it says\nand the second")))
        assertEquals("min was the letterbox bug", turnGist(proseTurn("a", "**min** was the letterbox bug")))
    }
}
