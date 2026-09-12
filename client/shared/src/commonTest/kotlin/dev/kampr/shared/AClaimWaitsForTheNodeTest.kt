package dev.kampr.shared

import dev.kampr.shared.ui.AppState
import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.ServerMsg
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
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
