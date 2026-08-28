package dev.kampr.conversation

import dev.kampr.shared.wire.Facets
import dev.kampr.shared.wire.Queued
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull
import kotlin.test.assertTrue

private const val AT = "2026-08-28T09:00:00.000Z"
// The second one runs to three lines on purpose: that is the length at which an ordinary turn
// earns a chevron, so it is the only fixture that can show a queued prompt does not.
private const val LONG = "run the tests\nand the lint\nand push the branch"
private val QUEUE = Facets(queued = listOf(Queued("stop what you are doing", AT), Queued(LONG)))

class QueuedTurnTest {
    @Test
    fun aHarnessWithNothingQueuedDrawsNothingAtAll() {
        assertTrue(queuedTurns(Facets()).isEmpty())
    }

    // It has to be tellable from a turn the harness has taken up: the operator's question is
    // whether the agent has their message, and two cards that look the same do not answer it.
    // Not foldable either, for the reason a live preview is not — a chevron on a card that is
    // about to be taken away is a control over nothing.
    @Test
    fun aQueuedPromptIsDrawnAsWaitingRatherThanAsARecordedTurn() {
        val waiting = queuedTurns(QUEUE)
        assertEquals(listOf("stop what you are doing", LONG), waiting.map(::turnText))
        for (turn in waiting) {
            assertEquals(Speaker.Queued, speakerOf(turn), "a queued prompt read as a turn the harness has")
            assertNull(foldKey(turn), "a card about to be taken away was given a chevron")
        }
        assertEquals(2, waiting.map { it.id }.toSet().size, "two cards under one id is one card")
    }

    // The enqueue stamp is the one thing the harness does record about a waiting prompt, and it
    // is the answer to "how long has this been sitting there".
    @Test
    fun aQueuedPromptKeepsTheStampTheHarnessWroteWhenItWasEnqueued() {
        assertEquals(AT, queuedTurns(QUEUE).first().at)
        assertNull(queuedTurns(QUEUE).last().at, "a prompt with no stamp was given one")
    }

    // The queue is a turn like any other to everything downstream of it, so it stands at the foot
    // of the transcript as the operator's own ask rather than joining the reply above it.
    @Test
    fun theQueueStandsAtTheFootOfTheTranscriptAsAsksOfTheirOwn() {
        val rows = transcriptRows(
            listOf(proseTurn("u-1", "what broke?", role = "user"), proseTurn("a-1", "The letterbox.")) +
                queuedTurns(QUEUE),
            "",
        )
        val tail = rows.takeLast(2)
        assertTrue(tail.all { it is TranscriptRow.Ask }, "the queue landed as ${tail.map { it::class.simpleName }}")
        assertEquals(listOf("stop what you are doing", LONG), tail.map { turnText(it.turns.single()) })
    }
}
