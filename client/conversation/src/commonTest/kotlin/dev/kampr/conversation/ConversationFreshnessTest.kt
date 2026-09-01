package dev.kampr.conversation

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.PaneState
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// The other half of the notice: what actually moves the flag. A page confirms the transcript, a
// dropped socket un-confirms it, and so does leaving the pane — because leaving stops the node's
// pump, and what is drawn after that is a memory until a fresh pump has paged again.
private const val PANE = "01JNODE.../w3:p2"

private const val PAGE =
    "{\"cursor\":\"a-1\",\"more\":false,\"pane\":\"$PANE\",\"t\":\"convo\",\"turns\":[" +
        "{\"at\":\"2026-08-20T09:00:01.000Z\",\"blocks\":[{\"b\":\"md\",\"text\":\"hello\"}]," +
        "\"id\":\"a-1\",\"role\":\"assistant\"}]}"

class ConversationFreshnessTest {
    private fun stored(): Pair<KamprStore, PaneState> {
        val store = KamprStore()
        store.accept(requireNotNull(Wire.decode(PAGE)) { "undecodable page" })
        return store to store.pane(PANE)
    }

    @Test
    fun aPageIsWhatConfirmsTheTranscript() {
        val (_, pane) = stored()
        assertTrue(pane.convoConfirmed, "a page landed and the transcript was still unconfirmed")
        assertTrue(pane.turns.isNotEmpty(), "the page carried no turns, so nothing was tested")
    }

    @Test
    fun aFreshPaneHasConfirmedNothing() {
        assertFalse(PaneState(PANE, dev.kampr.shared.model.StyleTable()).convoConfirmed)
    }

    // Every signal downstream of the socket dying, including this one. What is on screen was true
    // of a connection that is gone.
    @Test
    fun aDroppedSocketUnconfirmsWhatIsDrawn() {
        val (store, pane) = stored()
        store.markStale()
        assertFalse(pane.convoConfirmed, "the socket went away and the transcript still read as current")
    }

    // Leaving the pane stops the pump that was keeping it true, so reopening it starts from a
    // memory. This is the path the report came in on: away, back, and a conversation that had
    // moved on underneath.
    @Test
    fun leavingThePaneUnconfirmsItToo() {
        val (store, pane) = stored()
        store.noteConversationUnconfirmed(PANE)
        assertFalse(pane.convoConfirmed, "the pane was left and its transcript still read as current")
    }
}
