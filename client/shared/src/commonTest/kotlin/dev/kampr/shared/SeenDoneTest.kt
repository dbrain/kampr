package dev.kampr.shared

import dev.kampr.shared.model.AgentStatus
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.SeenDone
import dev.kampr.shared.model.statusOf
import dev.kampr.shared.model.withoutReadDone
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.wire.PaneInfo
import kotlin.test.Test
import kotlin.test.assertEquals

private fun pane(status: String, at: String? = "2026-09-01T03:00:00Z") = PaneInfo(
    id = "01JNODE/w1:p1",
    nodeId = "01JNODE",
    workspace = "kampr",
    tab = "1",
    agent = "claude",
    agentStatus = status,
    updatedAt = at,
)

private fun herdOf(vararg panes: PaneInfo) = Herd(panes = panes.toList(), known = true)

// herdr synthesises `done` for a pane that finished while nobody was looking, and it is the
// operator's unread flag. The one thing that clears herdr's own marker is focusing the pane, which
// destroys it for every other watcher too — so reading it here is a client-side fact and nothing
// in this file may reach the node.
class SeenDoneTest {
    @Test
    fun aPaneThatFinishedIsStillFlaggedUntilItHasBeenOpened() {
        val seen = SeenDone(MemoryPrefs())
        val finished = pane("done")
        assertEquals(
            AgentStatus.Done,
            statusOf(herdOf(finished).withoutReadDone(seen).panes.single()),
            "a pane that finished unread lost its flag before anybody looked at it",
        )
    }

    @Test
    fun openingThePaneIsWhatTakesTheFlagDown() {
        val seen = SeenDone(MemoryPrefs())
        val finished = pane("done")
        seen.saw(finished)
        assertEquals(
            AgentStatus.Idle,
            statusOf(herdOf(finished).withoutReadDone(seen).panes.single()),
            "the flag survived the operator opening the pane, so it never goes away",
        )
    }

    // The flag has to re-arm, or a pane finishes loudly once and is silent for the rest of the day.
    @Test
    fun theNextTimeThatPaneFinishesTheFlagIsRaisedAgain() {
        val seen = SeenDone(MemoryPrefs())
        seen.saw(pane("done", at = "2026-09-01T03:00:00Z"))
        val again = pane("done", at = "2026-09-01T04:00:00Z")
        assertEquals(
            AgentStatus.Done,
            statusOf(herdOf(again).withoutReadDone(seen).panes.single()),
            "a pane read once stays read for ever, however many times it finishes after",
        )
    }

    // A working pane is not something to mark read, and marking it would swallow the finish that
    // has not happened yet.
    @Test
    fun openingAPaneThatIsStillWorkingSwallowsNothing() {
        val seen = SeenDone(MemoryPrefs())
        seen.saw(pane("working"))
        val finished = pane("done")
        assertEquals(
            AgentStatus.Done,
            statusOf(herdOf(finished).withoutReadDone(seen).panes.single()),
            "opening a working pane pre-emptively read the finish it had not reached yet",
        )
    }

    // The browser is where this client is most often left open, and a reload is not a reason to be
    // told again about work the operator has already seen.
    @Test
    fun aReloadDoesNotRaiseEveryFlagAgain() {
        val prefs = MemoryPrefs()
        val finished = pane("done")
        SeenDone(prefs).saw(finished)
        assertEquals(
            AgentStatus.Idle,
            statusOf(herdOf(finished).withoutReadDone(SeenDone(prefs)).panes.single()),
            "every flag the operator had cleared came back on the next page load",
        )
    }

    // The transform is the whole point: one answer, so the sidebar's rank and the badge cannot
    // disagree and leave a read pane pinned to the top with nothing on it.
    @Test
    fun aHerdWithNothingReadIsHandedBackUntouched() {
        val seen = SeenDone(MemoryPrefs())
        val herd = herdOf(pane("done"), pane("working").copy(id = "01JNODE/w1:p2"))
        assertEquals(herd, herd.withoutReadDone(seen))
    }

    @Test
    fun aPaneTheHerdNoLongerCarriesIsForgotten() {
        val prefs = MemoryPrefs()
        val seen = SeenDone(prefs)
        seen.saw(pane("done"))
        seen.keep(emptySet())
        assertEquals(
            AgentStatus.Done,
            statusOf(herdOf(pane("done")).withoutReadDone(seen).panes.single()),
            "a pane that left the herd and came back was still remembered as read",
        )
    }
}
