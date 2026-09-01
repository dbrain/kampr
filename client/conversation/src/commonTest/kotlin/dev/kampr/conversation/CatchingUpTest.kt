package dev.kampr.conversation

import dev.kampr.shared.model.ConnectionStatus
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

// The transcript went out of date silently while the grid beside it did not (#393). The grid is
// warm — the registry holds a pane's stream across a re-watch (#252) — and the conversation is
// cold, so what is drawn after reopening a pane is a memory until the node has resolved, folded and
// paged.
//
// The words name an *edge* rather than a doubt, because the line they are drawn on is the foot of
// what this device has read. Everything above it was read off the transcript and is exactly right;
// only what comes after it is missing.
class CatchingUpTest {
    private val live = ConnectionStatus.Live("owner")

    @Test
    fun aConfirmedConversationOnALiveSocketSaysNothingAtAll() {
        assertNull(catchingUp(live, confirmed = true, drawn = true))
    }

    // An empty transcript is not out of date, it is empty — and it already says so in its own
    // words. Two notices about nothing is worse than one.
    @Test
    fun anEmptyTranscriptIsNotOutOfDate() {
        assertNull(catchingUp(live, confirmed = false, drawn = false))
        assertNull(catchingUp(ConnectionStatus.Offline("gone", 250), confirmed = false, drawn = false))
    }

    @Test
    fun turnsDrawnButNotYetConfirmedSayTheyAreCatchingUp() {
        assertEquals(
            "read up to here",
            catchingUp(live, confirmed = false, drawn = true),
        )
        assertEquals(
            "read up to here",
            catchingUp(ConnectionStatus.Connecting, confirmed = false, drawn = true),
        )
    }

    // A socket that is down is a different sentence: it is not catching up, it has nothing to catch
    // up with, and telling a reader to wait for something that is not coming is the worse lie.
    @Test
    fun anOfflineSocketSaysSoRatherThanPromisingAnUpdate() {
        assertEquals(
            "read up to here — offline",
            catchingUp(ConnectionStatus.Offline("connection closed", 250), confirmed = false, drawn = true),
        )
        assertEquals(
            "read up to here — offline",
            catchingUp(ConnectionStatus.Offline("connection closed", 250), confirmed = true, drawn = true),
            "a page confirmed on a socket that has since died is still only what it was",
        )
    }
}
