package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Block
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Turn
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE = "01JKAMPRNODE0000000000000/w1:p1"
private const val LIVE = "live"

private fun prose(id: String, text: String) =
    Turn(id = id, role = "assistant", blocks = listOf(Block.Md(text)))

private fun ConvoTurn(vararg turns: Turn) = ServerMsg.ConvoTurn(pane = PANE, sub = null, turns = turns.toList())

// The operator, watching a message arrive: *"the conversation pane when a message is being streamed
// — it seems to bounce all around the shop, editing parts of the message in the middle of the
// conversation"*.
//
// The preview a harness is painting is carried under one reserved id, revised as the text grows and
// withdrawn — same id, no blocks — when the record lands. A page merges by id, so the withdrawal
// *replaces* the preview where it sits rather than taking it out of the list, and every turn that
// arrives afterwards is filed below it. The next message's preview then re-uses that slot, which by
// then is in the middle of the conversation: the new message is drawn above turns that came before
// it, growing and shifting in place while the reader is somewhere else entirely.
//
// The preview is always the newest thing this pane has to say. Nothing else in a transcript moves,
// which is why this is a rule about the one id and not about ordering in general.
class LivePreviewPlacementTest {
    private fun visible(store: KamprStore) = store.pane(PANE).turns.filter { it.blocks.isNotEmpty() }.map { it.id }

    @Test
    fun aSecondMessagesPreviewIsDrawnAtTheEndRatherThanInTheSlotTheFirstOneLeft() {
        val store = KamprStore()
        store.accept(ConvoTurn(prose("a-1", "the first answer")))
        store.accept(ConvoTurn(prose(LIVE, "the second answer, half written")))
        assertEquals(listOf("a-1", LIVE), visible(store), "a preview opens at the end of the transcript")

        // The record lands and the preview is withdrawn, which is the shape a client is sent.
        store.accept(ConvoTurn(Turn(id = LIVE, role = "assistant")))
        store.accept(ConvoTurn(prose("a-2", "the second answer")))
        assertEquals(listOf("a-1", "a-2"), visible(store))

        store.accept(ConvoTurn(prose(LIVE, "the third answer, half written")))
        assertEquals(
            listOf("a-1", "a-2", LIVE),
            visible(store),
            "the preview was drawn in the slot the last one left, above messages older than it",
        )
    }

    // The same rule while the operator's own message is what arrives between two previews: a reply
    // sent from the phone is a turn like any other, and the preview of the answer to it belongs
    // under it rather than above it.
    @Test
    fun aPreviewFollowsTheMessageItIsAnAnswerTo() {
        val store = KamprStore()
        store.accept(ConvoTurn(prose(LIVE, "an answer being written")))
        store.accept(ConvoTurn(Turn(id = LIVE, role = "assistant")))
        store.accept(ConvoTurn(Turn(id = "u-1", role = "user", blocks = listOf(Block.Md("and now this")))))
        store.accept(ConvoTurn(prose(LIVE, "answering the new one")))
        assertEquals(
            listOf("u-1", LIVE),
            visible(store),
            "the preview of the answer was drawn above the message it is answering",
        )
    }
}
