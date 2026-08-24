package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNotNull
import kotlin.test.assertNull

private const val PANE = "01JNODE/w1:p1"

private fun herd(detail: String?): String {
    val field = detail?.let { ""","detail":"$it"""" } ?: ""
    return """{"t":"herd","nodes":[{"id":"01JNODE","name":"cc","kind":"local","online":true}],
              "panes":[{"id":"$PANE","node_id":"01JNODE","rows":30$field}]}"""
}

private const val REFUSAL =
    """{"t":"error","code":"stream_unavailable","pane":"$PANE",
       "message":"No pane on this node can show a screen: Kampr cannot run herdr."}"""

private fun KamprStore.take(frame: String) = accept(Wire.decode(frame) ?: error("undecodable: $frame"))

// A node can reach herdr over a socket and not over a spawned binary, and then it serves a right
// herd and no screens at all. The herd is where that state lives; the error frame is only its
// arrival.
class StreamFaultTest {
    @Test
    fun aPaneCarriesWhyItHasNoPicture() {
        val store = KamprStore()
        store.take(herd("Kampr cannot run herdr"))
        assertEquals("Kampr cannot run herdr", store.paneInfo(PANE)?.detail)
    }

    // Empty is an ordinary thing for a pane to be, and nothing about it is a fault.
    @Test
    fun aPaneThatCanBeStreamedCarriesNothing() {
        val store = KamprStore()
        store.take(herd(null))
        assertNull(store.paneInfo(PANE)?.detail)
    }

    @Test
    fun aPatchCarriesTheFaultAndTakesItAwayAgain() {
        val store = KamprStore()
        store.take(herd(null))
        store.take(
            """{"t":"herd.patch","changed":{"panes":[
               {"id":"$PANE","node_id":"01JNODE","rows":30,"detail":"cannot run herdr"}]}}"""
        )
        assertEquals("cannot run herdr", store.paneInfo(PANE)?.detail)
        store.take(
            """{"t":"herd.patch","changed":{"panes":[{"id":"$PANE","node_id":"01JNODE","rows":30}]}}"""
        )
        assertNull(store.paneInfo(PANE)?.detail, "the herd entry clearing is the recovery signal")
    }

    // The strip is dismissible and the fault is not, so the two have to be tied together at the
    // one end that can move on its own: an error nobody can act on any more must not sit there
    // telling an operator to fix a machine that is already fixed.
    @Test
    fun recoveryTakesTheErrorStripDownWithTheFault() {
        val store = KamprStore()
        store.take(herd("cannot run herdr"))
        store.take(REFUSAL)
        assertNotNull(store.failure.value, "the arrival is announced on the frame a v1 client knows")

        store.take(herd(null))
        assertNull(store.failure.value, "the strip outlived the fault it was about")
    }

    // Every other code is somebody's refusal of something they did, and stays until dismissed.
    @Test
    fun aHerdUpdateLeavesEveryOtherRefusalAlone() {
        val store = KamprStore()
        store.take("""{"t":"error","code":"not_writer","message":"this device is read-only"}""")
        store.take(herd(null))
        assertEquals("not_writer", store.failure.value?.code)
    }
}
