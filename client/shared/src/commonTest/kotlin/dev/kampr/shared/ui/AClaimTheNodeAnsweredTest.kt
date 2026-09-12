package dev.kampr.shared.ui

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.SizeMode
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

// The operator, on 0.1.80, on the wasm desktop: *"panes sometimes don't resize to fill the wasm
// desktop cols/rows ... i closed it the pane redrew and made claude some tiny little box ... didn't
// seem to recover until i manually set the size"*.
//
// A claim is the one op ADR 0012 lets reshape a pane, and it can be refused. Sending one and
// recording a hold was the same mistake in a different place from #233: the pane went on being
// reported as held at the view's size, the size it was actually left at was whatever the desk had,
// and the *only* trigger that would have asked again was a dedup test comparing the view against
// the size this client believed it had already got.
@OptIn(ExperimentalCoroutinesApi::class)
class AClaimTheNodeAnsweredTest {
    @Test
    fun a_claim_the_node_refused_is_not_remembered_as_a_hold() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        assertFalse(claim(holds, sent, A, 120, 40, take = false), "a refused claim answered as held")
        assertTrue(claim(holds, sent, A, 120, 40), "the refusal was taken for the hold it was not")

        assertEquals(
            listOf(SizeMode.Match, SizeMode.Match),
            sent.sizings(A).modes(),
            "the view asked again for a size it never got and was told it already had it: " +
                "${sent.sizings(A)}",
        )
    }

    // The other side of it, and the reason the belief exists at all: a pane already held at this
    // grid is not claimed again, because a re-claim supersedes the controller and herdr shows the
    // desk's own geometry in the gap.
    @Test
    fun a_claim_the_node_took_is_not_asked_for_twice() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        assertTrue(claim(holds, sent, A, 120, 40))
        assertTrue(claim(holds, sent, A, 120, 40))

        assertEquals(listOf(SizeMode.Match), sent.sizings(A).modes(), "${sent.sizings(A)}")
    }

    // An ack names the ask it answers and nothing else. Two panes claimed at once is an ordinary
    // pane switch, and matching answers to asks by arrival would hand one pane's refusal to the
    // other — which is the same wrong belief, arrived at from the other end.
    @Test
    fun an_ack_answers_the_pane_it_was_asked_about() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        val first = async { holds.claim(A, 120, 40) }
        runCurrent()
        val forA = sent.manages().last().rid
        val second = async { holds.claim(B, 90, 30) }
        runCurrent()
        val forB = sent.manages().last().rid

        holds.acked(refused(forB))
        holds.acked(took(forA))

        assertTrue(first.await(), "the pane that was taken was answered with another pane's refusal")
        assertFalse(second.await(), "the pane that was refused was answered with another pane's hold")
    }

    // A view that ends while its claim is still in flight still lets the pane go. The node answers
    // the two in the order they were sent, so the release lands on the hold the claim took — and
    // waiting for the ack before sending it would leave the pane held by nothing but the socket.
    @Test
    fun a_view_that_ends_mid_claim_still_gives_the_pane_back() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        val claim = async { holds.claim(A, 120, 40) }
        runCurrent()
        holds.release(A, linger = false)

        assertFalse(claim.await(), "a claim let go of on the way out answered as a hold")
        assertEquals(
            listOf(SizeMode.Match, SizeMode.Release),
            sent.sizings(A).modes(),
            "a pane claimed and then left behind was never given back: ${sent.sizings(A)}",
        )
    }
}
