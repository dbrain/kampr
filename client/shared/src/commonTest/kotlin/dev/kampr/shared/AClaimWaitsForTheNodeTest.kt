package dev.kampr.shared

import dev.kampr.shared.ui.AppState
import dev.kampr.shared.ui.MATCH_LINGER_MS
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNotNull
import kotlin.test.assertTrue

// The wiring, rather than either end of it: `MatchHolds` answers a claim off the ack, and the acks
// only reach it if the session is collecting them. Nothing else would say so — a collector that was
// never launched leaves every claim waiting for ever, which is a desk that holds nothing at all and
// a test of `MatchHolds` alone that passes anyway.
@OptIn(ExperimentalCoroutinesApi::class)
class AClaimWaitsForTheNodeTest {
    private val pane = "01JNODE/w1:p1"

    // The token the session put on its first ask. It is `MatchHolds`' own — the node echoes
    // whatever it was sent — and naming it here is what makes this a test of the wiring rather
    // than of either end.
    private val first = "match-1"

    @Test
    fun a_claim_is_answered_by_the_node_through_the_session() = runTest {
        val state = AppState(backgroundScope)

        val claim = async { state.claimMatch(pane, 120, 40) }
        runCurrent()
        state.store.accept(ServerMsg.Managed(op = ManageOp.PaneSize.OP, ok = true, id = null, rid = first))
        runCurrent()

        assertTrue(claim.await(), "the node took the pane and the session never heard the answer")
    }

    // The node's word that somebody else took the pane reaches the claims, through the codec and
    // the store, so the next claim at the same size goes to the node rather than being answered
    // from a belief the node has just contradicted.
    @Test
    fun a_hold_the_node_says_was_taken_is_asked_for_again() = runTest {
        val state = AppState(backgroundScope)
        val claim = async { state.claimMatch(pane, 120, 40) }
        runCurrent()
        state.store.accept(ServerMsg.Managed(op = ManageOp.PaneSize.OP, ok = true, id = null, rid = first))
        runCurrent()
        assertTrue(claim.await())

        state.store.accept(assertNotNull(Wire.decode("{\"t\":\"hold.lost\",\"pane\":\"$pane\",\"lease\":7}")))
        runCurrent()
        val again = async { state.claimMatch(pane, 120, 40) }
        runCurrent()
        assertFalse(again.isCompleted, "the pane was taken and the session answered from memory")
        state.store.accept(ServerMsg.Managed(op = ManageOp.PaneSize.OP, ok = true, id = null, rid = "match-2"))
        runCurrent()
        assertTrue(again.await())
    }

    // The app going out of sight reaches the claims, so what it held is let go of and the next
    // claim goes to the node.
    @Test
    fun a_pane_held_when_the_app_went_out_of_sight_is_asked_for_again() = runTest {
        val state = AppState(backgroundScope)
        val claim = async { state.claimMatch(pane, 120, 40) }
        runCurrent()
        state.store.accept(ServerMsg.Managed(op = ManageOp.PaneSize.OP, ok = true, id = null, rid = first))
        runCurrent()
        assertTrue(claim.await())

        state.store.looked(false)
        advanceTimeBy(MATCH_LINGER_MS + 1)
        runCurrent()
        state.store.looked(true)
        val again = backgroundScope.async { state.claimMatch(pane, 120, 40) }
        runCurrent()
        assertFalse(again.isCompleted, "a pane let go of while out of sight was answered from memory")
    }

    @Test
    fun a_refusal_reaches_the_view_that_asked() = runTest {
        val state = AppState(backgroundScope)

        val claim = async { state.claimMatch(pane, 120, 40) }
        runCurrent()
        state.store.accept(
            ServerMsg.Managed(
                op = ManageOp.PaneSize.OP,
                ok = false,
                id = null,
                rid = first,
                code = "herdr",
                message = "pane already has an attached client",
            ),
        )
        runCurrent()

        assertFalse(claim.await(), "a refused claim was reported to the view as a hold")
    }
}
