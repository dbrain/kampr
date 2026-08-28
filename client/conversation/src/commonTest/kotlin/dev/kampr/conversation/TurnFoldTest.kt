package dev.kampr.conversation

import dev.kampr.shared.util.parseIsoMillis
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

private const val AT = "2026-08-23T09:00:00.000Z"
private const val FLOATING = "2026-08-23T09:00:00.000"
private val NOW = requireNotNull(parseIsoMillis(AT))
private val FACE = Regex("^(\\d{2}:\\d{2}|[A-Z][a-z]{2} \\d{2}:\\d{2}|\\d{1,2} [A-Z][a-z]{2}( \\d{4})? \\d{2}:\\d{2})$")

class TurnFoldTest {
    // A chevron is a 36 dp target, so it has to buy more than it costs. Nothing here would save a
    // line by folding: a turn of nothing but calls is the run's business and wears a chevron
    // already, and a preview is being rewritten under the reader as they look at it.
    @Test
    fun nothingThatWouldNotSaveALineWearsAChevron() {
        val cases = mapOf(
            "a short reply" to proseTurn("u-1", "run the tests please", role = "user"),
            "one line" to proseTurn("a-1", "Done."),
            "one paragraph that merely wraps" to proseTurn("a-2", "The letterbox came from min where max was meant, and every pane on a phone showed it."),
            "a turn of calls" to bashTurn("a-3", "cargo test", "done", 3),
            "a preview" to proseTurn(LIVE_TURN_ID, "I will write the file,\nthen explain\nwhat it does."),
        )
        for ((what, turn) in cases) assertNull(foldKey(turn), "$what was given a chevron")
    }

    // A reply used to be excluded outright, because it sat in its own gutter on the right and was
    // short by construction. It is a full-width card now, and a pasted stack trace is a reply, so
    // the size test decides for whoever wrote it.
    @Test
    fun ananswerLongEnoughToBeWorthTheControlGetsOne() {
        assertEquals("fold:u-1", foldKey(proseTurn("u-1", "run the tests\nand the lint\nplease", role = "user")))
        assertEquals("fold:a-9", foldKey(proseTurn("a-9", "First.\n\nSecond.\n\nThird.")))
        assertNotNull(
            foldKey(Turn("a-10", "assistant", AT, listOf(Block.Md("here:"), Block.Code("bash", "ls")))),
            "prose and a fence are two pieces and fold together",
        )
    }

    // Which of the two forms a stamp takes, rather than what either one reads: a face is drawn in
    // the *runner's* zone, so pinning the characters here would pin the machine the suite runs on.
    // `TimeTest` owns the face itself, against a fixed offset.
    @Test
    fun aZonedStampIsATimeOfDayAndAZonelessOneIsAnAgeInstead() {
        for (now in listOf(NOW, NOW + 2 * 86_400_000, NOW + 400 * 86_400_000.0)) {
            val face = assertNotNull(turnStamp(AT, now), "a zoned stamp had no reading at all")
            assertTrue(FACE.matches(face), "a zoned stamp read as \"$face\"")
        }
        assertEquals("now", turnStamp(FLOATING, NOW))
        assertEquals("12m", turnStamp(FLOATING, NOW + 12 * 60_000))
        assertNull(turnStamp(null, NOW))
        assertNull(turnStamp("whenever", NOW))
    }

    // The harness's summary of the conversation it dropped is filed as a user record and rendered
    // as one, so the operator was shown three paragraphs in their own voice that they never typed
    // (#259). It is the one turn that starts shut, and the toggle set is a set of *departures*
    // from the default — so the same key that puts an answer away is the one that opens this.
    @Test
    fun aCompactionSummaryIsNotTheOperatorAndStartsShut() {
        val summary = summaryTurn("u-7")
        val answer = proseTurn("a-7", "First.\n\nSecond.\n\nThird.")

        assertEquals(Speaker.Summary, speakerOf(summary))
        assertEquals(Speaker.You, speakerOf(proseTurn("u-8", "run the tests", role = "user")))

        val key = assertNotNull(foldKey(summary), "a summary that cannot be opened is a summary lost")
        assertTrue(turnFolded(summary, emptyList()), "a summary nobody has touched is shut")
        assertTrue(!turnFolded(summary, listOf(key)), "and the toggle opens it")
        assertTrue(!turnFolded(answer, emptyList()), "while an answer nobody has touched is open")
        assertTrue(turnFolded(answer, listOf(requireNotNull(foldKey(answer)))), "and the toggle shuts it")
    }

    // Every compaction summary opens with the same sentence the harness writes every time, so the
    // first line of one names the boilerplate rather than this summary. A shut turn's one line has
    // to say what it is.
    @Test
    fun aShutSummaryNamesItselfRatherThanTheHarnessesOpeningSentence() {
        val gist = turnGist(summaryTurn("u-7"))
        assertTrue(!gist.startsWith("This session is being continued"), "the gist was the boilerplate: $gist")
        assertTrue(gist.isNotEmpty(), "a shut turn with nothing on its header is a blank row")
        assertEquals(gist, turnGist(summaryTurn("u-8", "Summary:\n- something else entirely")))
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
