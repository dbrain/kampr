package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

// What the herd says a machine costs is what a keystroke into one of its panes costs, which is the
// number an operator feels — not the node's ping to herdr, which is one leg of it.
class KeystrokeLatencyTest {
    @Test
    fun eachMachineIsShownTheMedianOfItsOwnRecentKeystrokes() {
        val store = KamprStore()
        assertNull(store.keystrokeMs.value["A"], "nothing typed is nothing measured")
        listOf(10L, 12L, 400L, 11L, 13L).forEach { store.echoed("A/w1:p1", it) }
        store.echoed("B/w1:p1", 90L)
        assertEquals(12.0, store.keystrokeMs.value["A"], "one stall is not what typing costs")
        assertEquals(90.0, store.keystrokeMs.value["B"], "another machine's keys are its own")
    }

    // The window moves: a machine that was slow an hour ago and is quick now reads quick.
    @Test
    fun anOldStretchOfTypingAgesOut() {
        val store = KamprStore()
        repeat(40) { store.echoed("A/w1:p1", 200L) }
        repeat(40) { store.echoed("A/w1:p1", 8L) }
        assertEquals(8.0, store.keystrokeMs.value["A"])
    }
}

class NodeLatencyLabelTest {
    private val node = dev.kampr.shared.wire.NodeInfo(id = "A", name = "comingclean")

    @Test
    fun aMachineNobodyHasTypedIntoIsShownItsPingAndCallsItOne() {
        assertEquals("ping 2.2 ms", dev.kampr.shared.ui.nodeLatency(node, emptyMap(), 2.2))
    }

    @Test
    fun onceAKeyHasComeBackItIsTheKeystrokeThatIsShown() {
        assertEquals("key 23 ms", dev.kampr.shared.ui.nodeLatency(node, mapOf("A" to 23.0), 2.2))
    }
}
