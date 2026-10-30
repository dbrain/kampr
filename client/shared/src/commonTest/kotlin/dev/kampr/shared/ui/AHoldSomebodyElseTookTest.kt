package dev.kampr.shared.ui

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.SizeMode
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.async
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runCurrent
import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

// The operator, on 0.1.128: a phone left in the background with a pane open, and the wasm desktop
// showing that pane *"tiny ... until bringing pane to foreground then background then foreground
// again"*. The phone's reconnect claimed the pane, the node handed it the hold, and this side went
// on believing it held the pane at the window's size, so it never asked again.
@OptIn(ExperimentalCoroutinesApi::class)
class AHoldSomebodyElseTookTest {
    @Test
    fun a_hold_the_node_says_was_taken_is_asked_for_again_and_its_view_is_told() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)
        val told = mutableListOf<String>()
        backgroundScope.launch { holds.lost.toList(told) }
        runCurrent()

        assertTrue(claim(holds, sent, A, 120, 40))
        holds.lost(A)
        runCurrent()
        assertEquals(listOf(A), told, "the view holding the pane was not told it lost it")
        assertTrue(claim(holds, sent, A, 120, 40))

        assertEquals(
            listOf(SizeMode.Match, SizeMode.Match),
            sent.sizings(A).modes(),
            "a pane somebody else took was believed still held, so it was never asked for again: " +
                "${sent.sizings(A)}",
        )
    }

    // The node acks a claim before it can say anything about the lease that claim was given, so a
    // loss heard while one is in flight is about the lease it replaces. Told, the view would let
    // go on its way past, and that release would land a linger later on the hold just taken.
    @Test
    fun a_loss_heard_while_a_claim_is_in_flight_does_not_unseat_it() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)
        val told = mutableListOf<String>()
        backgroundScope.launch { holds.lost.toList(told) }
        assertTrue(claim(holds, sent, A, 120, 40))

        val again = async { holds.claim(A, 130, 40) }
        runCurrent()
        holds.lost(A)
        holds.acked(took(sent.manages().last().rid))
        assertTrue(again.await())
        assertTrue(claim(holds, sent, A, 130, 40))

        assertEquals(2, sent.sizings(A).size, "${sent.sizings(A)}")
        assertEquals(emptyList(), told, "the view was told it lost the claim it was making")
    }

    // A phone put away is nobody looking, and its views cannot say so: Compose draws nothing while
    // the app is in the background, and a hidden tab gets no frames. So the session lets go of
    // what it holds, after the same linger a glance away costs nothing under.
    @Test
    fun everything_held_is_let_go_of_once_nobody_can_see_the_app() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)
        assertTrue(claim(holds, sent, A, 120, 40))
        assertTrue(claim(holds, sent, B, 120, 40))
        holds.hidden()
        advanceTimeBy(MATCH_LINGER_MS + 1)
        runCurrent()

        for (pane in listOf(A, B)) {
            assertEquals(listOf(SizeMode.Match, SizeMode.Release), sent.sizings(pane).modes(), "$pane: ${sent.sizings(pane)}")
        }
        assertTrue(claim(holds, sent, A, 120, 40))
        assertEquals(3, sent.sizings(A).size, "a pane let go of was answered from memory: ${sent.sizings(A)}")
    }

    @Test
    fun coming_back_inside_the_linger_lets_nothing_go() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)
        assertTrue(claim(holds, sent, A, 120, 40))
        holds.hidden()
        advanceTimeBy(MATCH_LINGER_MS / 2)
        assertTrue(holds.claim(A, 120, 40))
        advanceTimeBy(MATCH_LINGER_MS)
        runCurrent()

        assertEquals(listOf(SizeMode.Match), sent.sizings(A).modes(), "${sent.sizings(A)}")
    }

    // Nothing is let go of on the way out of a view whose hold was taken: the session has no
    // lease to scope the release to, and a release naming none lets go of whatever stands.
    @Test
    fun a_view_ending_after_its_hold_was_taken_sends_no_release() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)
        assertTrue(claim(holds, sent, A, 120, 40))
        holds.lost(A)
        holds.release(A)
        advanceTimeBy(MATCH_LINGER_MS + 1)
        runCurrent()

        assertEquals(listOf(SizeMode.Match), sent.sizings(A).modes(), "${sent.sizings(A)}")
    }
}
