package dev.kampr.conversation

import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private fun keys(rows: List<TranscriptRow>) = rows.map { it.key }

class ToolRunTest {
    // The report, verbatim: "conversations seem to list every single tool call and end up filling
    // the screen with tool calls". The screenshot behind it was six `Bash` cards with a sentence
    // of prose in the middle, and the prose is what makes it two runs.
    @Test
    fun aRunOfToolCallsBrokenByProseCollapsesIntoTwoRowsAndNotOne() {
        val rows = transcriptRows(TOOL_RUN_TURNS, "")
        assertEquals(listOf("r-1", "reply:r-2", "run:r-2", "r-5", "run:r-6"), keys(rows))
        val first = rows[2] as TranscriptRow.Run
        val second = rows[4] as TranscriptRow.Run
        assertEquals(listOf("Bash", "Bash", "Bash"), first.tools.map { it.name })
        assertEquals(listOf("Bash", "Read", "Bash"), second.tools.map { it.name })
    }

    // Collapsing two cards behind a row that says "2 tool calls" costs a tap and saves a line.
    @Test
    fun aRunTooShortToBeWorthATapIsLeftAsItWas() {
        for (calls in 1 until TOOL_RUN_MIN) {
            val turns = (1..calls).map { bashTurn("t-$it", "cargo test $it", "done", 3) }
            assertTrue(
                transcriptRows(turns, "").filter { it !is TranscriptRow.Head }.all { it is TranscriptRow.One },
                "$calls calls were collapsed behind a row",
            )
        }
        val enough = (1..TOOL_RUN_MIN).map { bashTurn("t-$it", "cargo test $it", "done", 3) }
        assertEquals(1, transcriptRows(enough, "").count { it is TranscriptRow.Run })
    }

    // A harness that batches its calls writes them into one record and the adapter carries that
    // through as one turn, so the run is counted in calls and not in turns.
    @Test
    fun aSingleTurnHoldingAWholeRunOfCallsIsStillARun() {
        val batched = Turn(
            "b-1", "assistant", null,
            (1..3).flatMap { listOf(Block.Tool("Bash", "cargo test $it", 3, "done"), Block.Code("bash", "cargo test $it")) },
        )
        val rows = transcriptRows(listOf(batched), "")
        assertEquals(3, (rows.filterIsInstance<TranscriptRow.Run>().single()).tools.size)
    }

    // A turn that speaks, a turn the reader wrote, and a turn that speaks *and* calls a tool are
    // all content between two calls — the last one because collapsing it would take its sentence
    // down with it.
    @Test
    fun anythingThatIsNotAToolCallEndsTheRun() {
        val enders = listOf(
            proseTurn("x", "a sentence"),
            Turn("x", "assistant", null, listOf(Block.Code("rust", "let a = 1;"))),
            Turn("x", "assistant", null, listOf(Block.Diff("a.rs", "@@\n-a\n+b\n"))),
            proseTurn("x", "do it again", role = "user"),
            Turn(
                "x", "assistant", null,
                listOf(Block.Tool("Bash", "five", 1, "done"), Block.Md("and here is why I ran that")),
            ),
        )
        for (ender in enders) {
            val turns = listOf(
                bashTurn("t-1", "one", "done", 1), bashTurn("t-2", "two", "done", 1),
                ender,
                bashTurn("t-3", "three", "done", 1), bashTurn("t-4", "four", "done", 1),
            )
            assertTrue(
                transcriptRows(turns, "").none { it is TranscriptRow.Run },
                "${ender.blocks.first()} left a run of two on either side of it collapsed",
            )
        }
    }

    // A match the counter promises and the screen hides is the defect this whole search exists to
    // avoid, so a run holding one is not a run.
    @Test
    fun aRunHoldingWhatTheSearchIsLookingForIsNotCollapsed() {
        assertEquals(
            listOf("r-1", "reply:r-2", "r-2", "r-3", "r-4", "r-5", "run:r-6"),
            keys(transcriptRows(TOOL_RUN_TURNS, "clippy")),
            "the run holding the clippy call stayed collapsed",
        )
        assertEquals(
            listOf("r-1", "reply:r-2", "run:r-2", "r-5", "r-6", "r-7", "r-8"),
            keys(transcriptRows(TOOL_RUN_TURNS, "kampr-node")),
            "the run holding the kampr-node call stayed collapsed",
        )
        assertEquals(
            listOf("r-1", "reply:r-2", "run:r-2", "r-5", "run:r-6"),
            keys(transcriptRows(TOOL_RUN_TURNS, "nothing at all")),
            "a query that matches nothing tore up every run on the screen",
        )
    }

    // Every hit the counter promises is a row the list can be aimed at, and no hit is inside a
    // collapsed one.
    @Test
    fun everyMatchIsARowOfItsOwn() {
        val rows = transcriptRows(TOOL_RUN_TURNS, "cargo test")
        val hits = searchHits(rows, "cargo test")
        assertEquals(listOf(4, 6, 8), hits)
        assertTrue(hits.all { rows[it] is TranscriptRow.One })
    }
}
