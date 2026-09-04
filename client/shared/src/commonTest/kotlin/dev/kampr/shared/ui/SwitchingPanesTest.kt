package dev.kampr.shared.ui

import dev.kampr.shared.wire.ClientMsg
import dev.kampr.shared.wire.ManageOp
import dev.kampr.shared.wire.SizeMode
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.advanceTimeBy
import kotlinx.coroutines.test.runTest
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val A = "01JKAMPRNODE0000000000000/w1:p1"
private const val B = "01JKAMPRNODE0000000000000/w1:p2"

// A pane far enough from the view that matching it is worth a reflow, so these tests are about the
// lease and not about the slack.
private const val PANE_COLS = 200
private const val PANE_ROWS = 60

private fun List<ClientMsg>.sizings(pane: String): List<ManageOp.PaneSize> =
    mapNotNull { (it as? ClientMsg.Manage)?.request as? ManageOp.PaneSize }.filter { it.at == pane }

private fun List<ManageOp.PaneSize>.modes() = map { it.mode }

// The operator, on 0.1.57: *"wasm desktop matches the view when its open - switching panes now
// bounces around. are we sending mixed sizes? are we switching the size back when leaving when
// switching again when we come back?"*
//
// Not mixed sizes: the same size, written and unwritten. A release restores the geometry the pane
// was found at (ADR 0013 point 3), so leaving a terminal view resized the pane it left and opening
// the next one resized that, and coming back did both again in reverse — two writes per switch,
// on a pane the operator is only looking at.
@OptIn(ExperimentalCoroutinesApi::class)
class SwitchingPanesTest {
    @Test
    fun switching_away_and_back_reshapes_neither_pane() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS)
        // To B, and back to A: each switch is one view leaving and one arriving.
        holds.release(A)
        holds.claim(B, 120, 40, PANE_COLS, PANE_ROWS)
        holds.release(B)
        holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS)
        advanceTimeBy(MATCH_LINGER_MS / 2)

        assertEquals(
            listOf(SizeMode.Match),
            sent.sizings(A).modes(),
            "the pane the operator came back to was put back and taken again: ${sent.sizings(A)}",
        )
        assertEquals(
            listOf(SizeMode.Match),
            sent.sizings(B).modes(),
            "the pane behind the switch was released while a return was still live: ${sent.sizings(B)}",
        )
    }

    // And a pane really left behind is given back, so the linger is a grace window rather than a
    // hold that never ends. A pane Kampr holds renders wrong at the desk (#18, #298), which is the
    // whole reason the release exists.
    @Test
    fun a_pane_left_behind_is_put_back_once_the_window_is_out() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS)
        holds.release(A)
        advanceTimeBy(MATCH_LINGER_MS / 2)
        assertEquals(listOf(SizeMode.Match), sent.sizings(A).modes(), "released early")

        advanceTimeBy(MATCH_LINGER_MS)
        assertEquals(
            listOf(SizeMode.Match, SizeMode.Release),
            sent.sizings(A).modes(),
            "a pane nobody came back to was never given up: ${sent.sizings(A)}",
        )
    }

    // The operator ticking the switch off is an answer about this pane, not a view ending, and it
    // is owed the pane back now rather than in twenty seconds.
    @Test
    fun turning_matching_off_gives_the_pane_back_at_once() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS)
        holds.release(A, linger = false)

        assertEquals(listOf(SizeMode.Match, SizeMode.Release), sent.sizings(A).modes())
    }

    // A window the operator dragged is a different grid, and that is a claim rather than a repeat.
    @Test
    fun a_window_that_changed_size_claims_again() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS)
        holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS)
        holds.claim(A, 160, 50, PANE_COLS, PANE_ROWS)

        val asked = sent.sizings(A)
        assertEquals(listOf(SizeMode.Match, SizeMode.Match), asked.modes(), "$asked")
        assertEquals(160, asked.last().cols)
    }

    // And the decision is taken once, on the edge where the view first asks — never again while
    // the hold stands. Re-asking would answer "close enough" (the pane is now exactly the view's
    // size), release it, and the restore would put it back out of range: an oscillator.
    @Test
    fun a_pane_already_held_is_not_re_judged_against_the_slack() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        assertTrue(holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS), "the first claim was declined")
        // The pane has arrived at the view's size, which is what a slack test would call
        // "close enough".
        assertTrue(holds.claim(A, 120, 40, 120, 40), "the hold was dropped once the pane fitted")
        assertEquals(listOf(SizeMode.Match), sent.sizings(A).modes(), "${sent.sizings(A)}")
    }

    // A pane close enough to begin with is never claimed at all, so nothing is held and nothing is
    // released — the strip must not say a pane is held either.
    @Test
    fun a_pane_close_enough_to_begin_with_is_never_taken() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        assertTrue(!holds.claim(A, 289, 69, 292, 72), "it claimed a pane it did not need to")
        holds.release(A)
        advanceTimeBy(MATCH_LINGER_MS * 2)

        assertTrue(sent.sizings(A).isEmpty(), "it wrote to a pane it never held: ${sent.sizings(A)}")
    }

    // The operator's own numbers, off the audit log: every one of 31 claims asked for 289x69
    // against a pane that was 292x72. Three columns and three rows, and the price is a PTY reflow
    // plus an observe child torn down and restarted — twice, because the release does it again.
    @Test
    fun a_pane_already_this_close_to_the_view_is_not_worth_a_reflow() {
        assertTrue(
            !worthMatching(viewCols = 289, viewRows = 69, paneCols = 292, paneRows = 72),
            "the operator's own case: 1% of the width, for four disturbances a round trip",
        )
    }

    // And a pane that really is the wrong shape still gets matched, which is the whole feature.
    @Test
    fun a_pane_the_wrong_shape_for_the_view_is_still_claimed() {
        assertTrue(worthMatching(120, 40, 200, 50), "a much wider pane went unmatched")
        assertTrue(worthMatching(120, 40, 80, 24), "a much narrower pane went unmatched")
        assertTrue(worthMatching(120, 40, 120, 60), "twenty rows out and still not worth it")
        assertTrue(
            worthMatching(120, 40, 0, 0),
            "a pane whose grid is not known yet has to be claimed, not assumed to fit",
        )
    }

    // The socket going takes every lease with it and restores every pane, at the node. A session
    // that went on believing it held them would claim nothing the next time a view asked.
    @Test
    fun a_dropped_socket_leaves_nothing_believed_held() = runTest {
        val sent = mutableListOf<ClientMsg>()
        val holds = MatchHolds(backgroundScope, sent::add)

        holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS)
        holds.disconnected()
        holds.claim(A, 120, 40, PANE_COLS, PANE_ROWS)
        advanceTimeBy(MATCH_LINGER_MS * 2)

        assertEquals(listOf(SizeMode.Match, SizeMode.Match), sent.sizings(A).modes())
        assertTrue(
            sent.sizings(A).none { it.mode == SizeMode.Release },
            "a release was sent down a socket that had already gone: ${sent.sizings(A)}",
        )
    }
}
