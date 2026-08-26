package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Cursor
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import kotlin.test.Test
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

private fun info(
    scrollbackRows: Int = 0,
    agentStatus: String = "idle",
) = PaneInfo(id = PANE, nodeId = "01JNODE", scrollbackRows = scrollbackRows, agentStatus = agentStatus)

private fun herd(info: PaneInfo) = ServerMsg.Herd(nodes = emptyList(), panes = listOf(info))

private fun reset() = ServerMsg.GridReset(
    pane = PANE,
    cols = 4,
    rows = 1,
    rowsData = emptyList(),
    cursor = Cursor(0, 0, true),
    links = emptyList(),
)

// The report this exists for: a browser pane frozen on a screen minutes old while the socket was
// up, the herd list was fresh, and that same pane's own conversation was answering over that same
// socket. Nothing in the client could see it — `stale` only ever meant "the socket went away", so
// every surface said the pane was fine.
//
// What it watches is not silence, which an idle pane produces for hours. It is the node's two
// halves disagreeing: the socket plane keeps reporting the pane working while the stream plane
// delivers nothing. They fail independently (#233), and this is what that looks like from here.
class QuietPaneTest {
    private fun store(): KamprStore = KamprStore().apply {
        accept(herd(info()))
        accept(reset())
    }

    @Test
    fun `a pane the node keeps reporting active while sending no frames is called quiet`() {
        val store = store()
        val pane = store.pane(PANE)
        assertFalse(pane.quiet, "a pane that has just painted was already being accused")

        // Three sweeps of the herd, each one saying this pane wrote lines. No frame in between.
        store.accept(herd(info(scrollbackRows = 10)))
        assertFalse(pane.quiet, "one sweep is a race the grid frame can lose fairly")
        store.accept(herd(info(scrollbackRows = 20)))
        assertFalse(pane.quiet, "two sweeps is still inside the margin")
        store.accept(herd(info(scrollbackRows = 30)))
        assertTrue(pane.quiet, "the node reported this pane working three times over and sent nothing")
    }

    // The property the whole thing rests on. A pane nobody is typing in reports the same readings
    // for as long as it is left alone, so it can never accumulate a single count — which is why
    // this is safe to run against every pane on screen with no timer anywhere in it.
    @Test
    fun `a pane nobody is typing in is never accused however long it sits there`() {
        val store = store()
        repeat(500) { store.accept(herd(info())) }
        assertFalse(store.pane(PANE).quiet, "an idle pane was called quiet for being idle")
    }

    // A full-screen agent keeps no scrollback at all, so its ring never grows — and this is the
    // pane the report was actually about. Its status is the half that moves.
    @Test
    fun `an alt screen agent with no scrollback is still seen to be working`() {
        val store = store()
        store.accept(herd(info(agentStatus = "working")))
        store.accept(herd(info(agentStatus = "idle")))
        store.accept(herd(info(agentStatus = "working")))
        assertTrue(store.pane(PANE).quiet, "a pane with no ring to grow could never be noticed")
    }

    @Test
    fun `one frame arriving clears the whole count`() {
        val store = store()
        repeat(3) { store.accept(herd(info(scrollbackRows = it * 10 + 10))) }
        assertTrue(store.pane(PANE).quiet)
        store.accept(reset())
        assertFalse(store.pane(PANE).quiet, "a pane that answered was still being called quiet")
    }

    // The node knows this outright when its own registry notices the feeder is gone, and says so
    // — but the notice does not survive: `dropRepairedFault` takes a `stream_unavailable` down as
    // soon as the pane's herd entry carries no `detail`, and a per-pane stream death never sets
    // one. So the sweep three seconds later would clear the only thing that had been said.
    @Test
    fun `the node saying the stream stopped outlives the next herd sweep`() {
        val store = store()
        store.accept(ServerMsg.Failure("stream_unavailable", "frames stopped", PANE))
        assertTrue(store.pane(PANE).quiet, "the node said the stream stopped and nothing recorded it")
        store.accept(herd(info()))
        assertTrue(store.pane(PANE).quiet, "one herd sweep took the only notice back down")
        store.accept(reset())
        assertFalse(store.pane(PANE).quiet, "the pane answered and was still being accused")
    }

    // `stale` is the socket having gone away, and that is a different and louder thing that is
    // already said. Saying both at once is two badges for one fact.
    @Test
    fun `a pane on a dead socket is stale rather than quiet`() {
        val store = store()
        repeat(3) { store.accept(herd(info(scrollbackRows = it * 10 + 10))) }
        store.markStale()
        assertFalse(store.pane(PANE).quiet, "a pane whose socket died was accused of going quiet too")
    }
}
