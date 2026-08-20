package dev.kampr.mosaic

import androidx.compose.ui.unit.dp
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertTrue

private fun state(): Pair<MosaicState, MemoryPrefs> {
    val prefs = MemoryPrefs()
    val connection = KamprConnection(CoroutineScope(Job()), KamprStore())
    return MosaicState(prefs, connection) to prefs
}

private fun pane(id: String, node: String) = PaneInfo(id = id, nodeId = node)

class MosaicLayoutTest {
    @Test
    fun theShapeFollowsTheCountAndTheWindow() {
        val cases = listOf(
            Triple(1, 1440.dp, listOf(1)),
            Triple(2, 1440.dp, listOf(2)),
            Triple(3, 1440.dp, listOf(2, 1)),
            Triple(4, 1440.dp, listOf(2, 2)),
            Triple(2, 760.dp, listOf(2)),
            Triple(2, 700.dp, listOf(1, 1)),
            Triple(3, 700.dp, listOf(1, 1, 1)),
            Triple(4, 420.dp, listOf(1, 1, 1, 1)),
        )
        for ((count, width, expected) in cases) {
            assertEquals(expected, mosaicShape(count, width).perRow, "$count panes at $width")
        }
    }

    @Test
    fun theMosaicNeverGrowsPastFourCells() {
        assertEquals(MAX_CELLS, mosaicShape(9, 1440.dp).cells)
        assertEquals(MAX_CELLS, decodeArrangement("a b c d e f").size)
    }

    @Test
    fun anArrangementSurvivesEncoding() {
        val panes = listOf("n1/p1", "n2/p2", "n1.agents/p3")
        assertEquals(panes, decodeArrangement(encodeArrangement(panes)))
        assertEquals(emptyList(), decodeArrangement(null))
        assertEquals(emptyList(), decodeArrangement(""))
    }
}

class MosaicStateTest {
    @Test
    fun addingAndRemovingMovesTheFocusAndTheWatches() {
        val (mosaic, _) = state()
        mosaic.attach()
        mosaic.add("n1/p1")
        mosaic.add("n2/p2")
        assertEquals(listOf("n1/p1", "n2/p2"), mosaic.panes)
        assertEquals("n2/p2", mosaic.focused)
        assertEquals(2, mosaic.observers)

        mosaic.focus("n1/p1")
        mosaic.remove("n1/p1")
        assertEquals(listOf("n2/p2"), mosaic.panes)
        assertEquals("n2/p2", mosaic.focused)
        assertEquals(1, mosaic.observers)
    }

    @Test
    fun aPaneIsNeverAddedTwiceAndNeverPastTheCap() {
        val (mosaic, _) = state()
        repeat(2) { mosaic.add("n1/p1") }
        for (i in 2..6) mosaic.add("n1/p$i")
        assertEquals(MAX_CELLS, mosaic.panes.size)
        assertEquals(mosaic.panes.distinct(), mosaic.panes)
        assertTrue(mosaic.full)
    }

    @Test
    fun savingIsExplicitAndSurvivesARestore() {
        val (mosaic, prefs) = state()
        mosaic.add("n1/p1")
        assertFalse(mosaic.saved)
        mosaic.save()
        assertTrue(mosaic.saved)
        mosaic.add("n2/p2")
        assertFalse(mosaic.saved, "changing the arrangement makes it savable again")
        mosaic.remove("n2/p2")
        assertTrue(mosaic.saved, "and undoing the change makes it saved again")

        val restored = MosaicState(prefs, KamprConnection(CoroutineScope(Job()), KamprStore()))
        restored.restore()
        assertEquals(listOf("n1/p1"), restored.panes)
        assertEquals("n1/p1", restored.focused)
    }

    @Test
    fun steppingWrapsInBothDirections() {
        val (mosaic, _) = state()
        for (i in 1..3) mosaic.add("n1/p$i")
        mosaic.focus("n1/p1")
        mosaic.step(-1)
        assertEquals("n1/p3", mosaic.focused)
        mosaic.step(1)
        assertEquals("n1/p1", mosaic.focused)
    }

    // A dropped peer's panes stay in the herd on purpose, so the cells stay too and degrade in
    // place; only a pane that has genuinely gone loses its cell.
    @Test
    fun anOfflinePeerKeepsItsCellsAndAClosedPaneDoesNot() {
        val (mosaic, _) = state()
        mosaic.attach()
        mosaic.add("hub/p1")
        mosaic.add("peer/p2")
        mosaic.reconcile(
            Herd(
                nodes = listOf(
                    NodeInfo(id = "hub", kind = "local"),
                    NodeInfo(id = "peer", online = false, detail = "peer is not connected: eof"),
                ),
                panes = listOf(pane("hub/p1", "hub"), pane("peer/p2", "peer")),
                known = true,
            )
        )
        assertEquals(listOf("hub/p1", "peer/p2"), mosaic.panes)

        mosaic.reconcile(
            Herd(
                nodes = listOf(NodeInfo(id = "hub", kind = "local")),
                panes = listOf(pane("hub/p1", "hub")),
                known = true,
            )
        )
        assertEquals(listOf("hub/p1"), mosaic.panes)
        assertEquals("hub/p1", mosaic.focused)
    }

    @Test
    fun anUnknownHerdNeverEmptiesTheMosaic() {
        val (mosaic, _) = state()
        mosaic.add("n1/p1")
        mosaic.reconcile(Herd())
        assertEquals(listOf("n1/p1"), mosaic.panes)
    }
}

class CellInputTest {
    private class Base(override val readOnly: Boolean) : dev.kampr.shared.ui.PaneIo {
        val sent = mutableListOf<dev.kampr.shared.wire.ClientMsg>()
        override fun send(msg: dev.kampr.shared.wire.ClientMsg) {
            sent += msg
        }
        override fun prefs(paneId: String) = dev.kampr.shared.wire.PanePrefs()
    }

    // Input reaches exactly one cell, and a read-only device reaches none — refused the same way,
    // so the key row and the destructive-command guard both follow the focus without knowing it.
    @Test
    fun onlyTheFocusedCellOfAWritableDeviceTakesInput() {
        val writable = Base(readOnly = false)
        assertFalse(CellIo(writable, writable = true).readOnly, "the focused cell is writable")
        assertTrue(CellIo(writable, writable = false).readOnly, "an unfocused cell is not")

        val readonlyDevice = Base(readOnly = true)
        assertTrue(CellIo(readonlyDevice, writable = true).readOnly, "a readonly device is refused every cell")
        assertTrue(CellIo(readonlyDevice, writable = false).readOnly)
    }
}

class NodeNamingTest {
    // A named session is its own herdr server and joins as its own node, named `<host>/<session>`.
    @Test
    fun aNodeNameSplitsIntoHostAndSession() {
        val primary = NodeInfo(id = "01J", name = "comingclean")
        val named = NodeInfo(id = "01J.agents", name = "comingclean/agents")
        assertEquals("comingclean" to "default", primary.host to primary.session)
        assertEquals("comingclean" to "agents", named.host to named.session)
    }
}

class WatchOwnerTest {
    // Two viewers, one stream: the last one to let go is the one that stops it.
    @Test
    fun aPaneStaysWatchedUntilEveryViewerLetsGo() {
        val connection = KamprConnection(CoroutineScope(Job()), KamprStore())
        connection.watch("n1/p1", "screen")
        connection.watch("n1/p1", "mosaic")
        assertEquals(setOf("n1/p1"), connection.observedPanes())
        connection.unwatch("n1/p1", "screen")
        assertEquals(setOf("n1/p1"), connection.observedPanes())
        connection.unwatch("n1/p1", "mosaic")
        assertEquals(emptySet(), connection.observedPanes())
    }
}
