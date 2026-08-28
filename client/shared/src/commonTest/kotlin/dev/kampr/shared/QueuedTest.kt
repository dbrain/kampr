package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

private fun decode(frame: String): ServerMsg = Wire.decode(frame) ?: error("undecodable: $frame")

private const val QUEUE = """{"t":"convo.facets","pane":"$PANE","facets":{"queued":[
    {"text":"run the tests again please","at":"2026-08-28T09:00:00.000Z"},
    {"text":"and the lint"}]}}"""

class QueuedTest {
    // The defect: a message sent while the harness is mid-turn is queued, and nothing the
    // conversation held said so — the terminal pane showed the text and the transcript showed
    // nothing at all until the harness reached it, which on a long turn is minutes.
    @Test
    fun thePromptsAHarnessHasQueuedReachTheClientItIsDrawnFor() {
        val store = KamprStore()
        store.accept(decode(QUEUE))
        val queued = store.pane(PANE).facets.queued
        assertEquals(listOf("run the tests again please", "and the lint"), queued.map { it.text })
        assertEquals("2026-08-28T09:00:00.000Z", queued.first().at)
        assertTrue(queued.last().at == null, "a queue entry with no stamp is not one with an empty stamp")
    }

    // The node folds the queue on the tail and republishes whenever it moves, so the newest frame
    // is the queue — merging one into what is held would leave a delivered prompt standing for
    // ever, which is the defect the node's own fold was written to avoid (#320).
    @Test
    fun theNewestFacetsReplaceWhatIsHeldRatherThanMergingIntoIt() {
        val store = KamprStore()
        store.accept(decode(QUEUE))
        store.accept(decode("""{"t":"convo.facets","pane":"$PANE","facets":{"queued":[{"text":"and the lint"}]}}"""))
        assertEquals(listOf("and the lint"), store.pane(PANE).facets.queued.map { it.text })
        store.accept(decode("""{"t":"convo.facets","pane":"$PANE","facets":{}}"""))
        assertTrue(store.pane(PANE).facets.queued.isEmpty(), "a drained queue left its last prompt standing")
    }

    // Every facet is optional and three harnesses fill different ones, so a frame carrying facets
    // this client does not model — and an empty one — has to land rather than be dropped whole.
    @Test
    fun aHarnessWithNothingToSayIsStillAFrameThatLands() {
        assertTrue(decode("""{"t":"convo.facets","pane":"$PANE","facets":{}}""") is ServerMsg.ConvoFacets)
        val other = decode(
            """{"t":"convo.facets","pane":"$PANE","facets":{"title":{"text":"the mesh","source":"generated"},
                "timings":[{"turn":"a-1","duration_ms":4200}],"queued":[{"text":"stop"}]}}""",
        )
        assertEquals(listOf("stop"), (other as ServerMsg.ConvoFacets).facets.queued.map { it.text })
    }

    // A `convo.facets` naming no pane is not about anything.
    @Test
    fun facetsWithNoPaneAreNotAFrame() {
        assertTrue(Wire.decode("""{"t":"convo.facets","facets":{"queued":[{"text":"stop"}]}}""") == null)
    }

    // The queue belongs to the pane, not to the conversation the reader happens to have open, and
    // it is what the *harness* recorded — so a device that may not type still sees what is
    // waiting, and so does a device that did not send any of it.
    @Test
    fun theQueueIsThePanesStateAndNotThisClientsOwn() {
        val store = KamprStore()
        store.accept(decode(QUEUE))
        store.accept(decode("""{"t":"role","role":"readonly"}"""))
        assertTrue(store.readOnly)
        assertEquals(2, store.pane(PANE).facets.queued.size)
    }
}
