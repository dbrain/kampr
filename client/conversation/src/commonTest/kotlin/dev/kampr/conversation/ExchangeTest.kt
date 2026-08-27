package dev.kampr.conversation

import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull
import kotlin.test.assertTrue

private fun keys(rows: List<TranscriptRow>) = rows.map { it.key }

private val EXCHANGE = listOf(
    proseTurn("u-1", "check the tests and the lint", role = "user"),
    proseTurn("a-2", "Running them now."),
    bashTurn("a-3", "cargo fmt --all -- --check", "done", 1),
    proseTurn("a-4", "Clippy is unhappy. Do you want me to fix it?"),
    proseTurn("u-5", "yes please", role = "user"),
    proseTurn("a-6", "Done."),
)

class ExchangeTest {
    // The unit the reader thinks in. Everything the agent did between one thing the operator said
    // and the next is one block with one head, however many records the harness wrote for it.
    @Test
    fun everythingBetweenTwoThingsTheOperatorSaidIsOneBlock() {
        val rows = transcriptRows(EXCHANGE, "")
        assertEquals(listOf("u-1", "reply:a-2", "a-2", "a-3", "a-4", "u-5", "reply:a-6", "a-6"), keys(rows))
        val first = rows[1] as TranscriptRow.Head
        assertEquals(listOf("a-2", "a-3", "a-4"), first.reply.turns.map { it.id })
        assertEquals(3, first.reply.steps)
        assertEquals(1, first.reply.tools)
        assertTrue(rows.drop(2).take(3).all { it.block == "reply:a-2" }, "a step named a block it is not in")
    }

    // One box per block, drawn a piece at a time: the head opens it, the steps continue it and the
    // last of them closes it. A piece that got its edge wrong draws a lid across the middle of a
    // reply or leaves the box hanging open into the next one.
    @Test
    fun thePiecesOfABlockAgreeOnWhichBoxTheyAreIn() {
        val rows = transcriptRows(EXCHANGE, "")
        val edges = rows.indices.map {
            blockEdge(rows.getOrNull(it - 1)?.block, rows[it].block, rows.getOrNull(it + 1)?.block)
        }
        assertEquals(
            listOf(
                BlockEdge.Only,                                    // the ask
                BlockEdge.Head,                                    // the reply opens its box
                BlockEdge.Middle, BlockEdge.Middle, BlockEdge.Foot, // its three steps close it
                BlockEdge.Only,                                    // the second ask
                BlockEdge.Head, BlockEdge.Foot,                    // a reply of one step
            ),
            edges,
        )
    }

    // A reply nobody has opened is one piece, so it is a box with a top and a bottom and no middle.
    @Test
    fun aPutAwayReplyIsABoxOfItsOwn() {
        val rows = transcriptRows(EXCHANGE, "", listOf("reply:a-2"))
        assertEquals(BlockEdge.Only, blockEdge(rows[0].block, rows[1].block, rows[2].block))
    }

    // The whole ask: one tap takes an answer and every tool call in it off the screen, and leaves
    // the head behind so the reader can see what they put away and bring it back.
    @Test
    fun puttingAReplyAwayTakesEveryStepOfItWithIt() {
        val rows = transcriptRows(EXCHANGE, "", listOf("reply:a-2"))
        assertEquals(listOf("u-1", "reply:a-2", "u-5", "reply:a-6", "a-6"), keys(rows))
    }

    // A transcript paged backwards opens in the middle of a reply, and a reply with nothing in
    // front of it is still a reply — not a block with no head, which is a block nothing can put
    // away.
    @Test
    fun aReplyWithNoAskInFrontOfItIsStillABlock() {
        val rows = transcriptRows(EXCHANGE.drop(1), "")
        assertEquals(listOf("reply:a-2", "a-2", "a-3", "a-4", "u-5", "reply:a-6", "a-6"), keys(rows))
    }

    // Two things said in a row are two asks. Neither of them opens a reply, and an empty block
    // would be a head with a chevron that hides nothing.
    @Test
    fun twoThingsSaidInARowAreTwoAsksAndNoEmptyReply() {
        val rows = transcriptRows(
            listOf(proseTurn("u-1", "wait", role = "user"), proseTurn("u-2", "actually, carry on", role = "user")),
            "",
        )
        assertEquals(listOf("u-1", "u-2"), keys(rows))
    }

    // Same rule the runs got: a match the counter promises and the screen hides is worse than a
    // screen that is too long, so a put-away reply holding one opens itself.
    @Test
    fun aPutAwayReplyHoldingWhatTheSearchWantsOpensItself() {
        val away = listOf("reply:a-2")
        assertEquals(listOf("u-1", "reply:a-2", "u-5", "reply:a-6", "a-6"), keys(transcriptRows(EXCHANGE, "", away)))
        assertTrue(
            "a-3" in keys(transcriptRows(EXCHANGE, "clippy", away)),
            "a put-away reply held the match and stayed shut",
        )
    }

    // A head is not a row a search can land on. It carries every turn of its reply, so counting it
    // would count the reply twice and step the reader through the same match twice.
    @Test
    fun aReplyHeadIsNotAMatchOfItsOwn() {
        val rows = transcriptRows(EXCHANGE, "clippy")
        val hits = searchHits(rows, "clippy")
        assertEquals(1, hits.size, "the head and the step both answered for one match")
        assertTrue(rows[hits.single()] is TranscriptRow.One)
    }
}

private val SAME_MINUTE = listOf(
    proseTurn("u-1", "run it", role = "user"),
    Turn("a-2", "assistant", "2026-08-20T09:00:04Z", listOf(Block.Md("Running it now."))),
    Turn("a-3", "assistant", "2026-08-20T09:00:41Z", listOf(Block.Md("Still going."))),
    Turn("a-4", "assistant", "2026-08-20T09:02:11Z", listOf(Block.Md("Done."))),
    Turn("a-5", "assistant", null, listOf(Block.Md("And here is why."))),
)

class StepStampTest {
    // Four records inside one minute carry one stamp between them, and the head has already said
    // when the reply began — so the step that landed in that same minute says nothing either. What
    // is left is the line a reader can actually use: the moments the clock moved.
    @Test
    fun aStampThatRepeatsTheLineAboveItIsNotDrawn() {
        val rows = transcriptRows(SAME_MINUTE, "")
        val now = requireNotNull(dev.kampr.shared.util.parseIsoMillis("2026-08-20T09:05:00Z"))
        val stamps = stepStamps(rows, now)
        assertEquals(listOf(null, null, null, null), stamps.take(4), "the head or its first minute repeated itself")
        assertEquals(1, stamps.count { it != null }, "more than the one moment the clock moved was drawn")
        assertNotNull(stamps[4], "the step that moved the clock lost its stamp")
    }

    // A step the harness left unstamped must not silently inherit the one above it — and must not
    // reset what the *next* step is measured against either.
    @Test
    fun anUnstampedStepNeitherBorrowsAStampNorClearsTheLastOne() {
        val rows = transcriptRows(SAME_MINUTE, "")
        val now = requireNotNull(dev.kampr.shared.util.parseIsoMillis("2026-08-20T09:05:00Z"))
        assertNull(stepStamps(rows, now).last(), "an unstamped step was given the one above it")
    }

    // Each block is measured on its own. A reply opening in the same minute the one before it
    // closed is still a new block, and its head has said so.
    @Test
    fun aNewBlockStartsTheMeasurementAgain() {
        val turns = SAME_MINUTE + listOf(
            proseTurn("u-6", "thanks", role = "user"),
            Turn("a-7", "assistant", "2026-08-20T09:02:11Z", listOf(Block.Md("Any time."))),
        )
        val rows = transcriptRows(turns, "")
        val now = requireNotNull(dev.kampr.shared.util.parseIsoMillis("2026-08-20T09:05:00Z"))
        val stamps = stepStamps(rows, now)
        assertNull(stamps.last(), "the second reply's first step repeated its own head")
    }
}
