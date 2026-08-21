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

class MosaicOrderTest {
    private fun four(): MosaicState {
        val (mosaic, _) = state()
        mosaic.attach()
        for (i in 1..4) mosaic.add("n$i/p$i")
        return mosaic
    }

    @Test
    fun aPaneMovesToAPositionAndTheRestCloseUpBehindIt() {
        val mosaic = four()
        mosaic.move("n4/p4", 0)
        assertEquals(listOf("n4/p4", "n1/p1", "n2/p2", "n3/p3"), mosaic.panes)
        mosaic.move("n4/p4", 2)
        assertEquals(listOf("n1/p1", "n2/p2", "n4/p4", "n3/p3"), mosaic.panes)

        // Out of range is a clamp, not a crash: a drag ends wherever the finger left the window.
        mosaic.move("n1/p1", 99)
        assertEquals(listOf("n2/p2", "n4/p4", "n3/p3", "n1/p1"), mosaic.panes)
        mosaic.move("n1/p1", -3)
        assertEquals(listOf("n1/p1", "n2/p2", "n4/p4", "n3/p3"), mosaic.panes)
        mosaic.move("nobody/p9", 0)
        assertEquals(listOf("n1/p1", "n2/p2", "n4/p4", "n3/p3"), mosaic.panes)
    }

    // The keyboard and screen-reader path: one step at a time, and the ends are a stop rather than
    // a wrap — wrapping a four-cell grid is disorienting when you cannot see it move.
    @Test
    fun aPaneStepsOnePlaceAtATimeAndStopsAtTheEnds() {
        val mosaic = four()
        assertTrue(mosaic.moveBy("n1/p1", 1))
        assertEquals(listOf("n2/p2", "n1/p1", "n3/p3", "n4/p4"), mosaic.panes)
        assertFalse(mosaic.moveBy("n2/p2", -1), "the first cell cannot move earlier")
        assertFalse(mosaic.moveBy("n4/p4", 1), "the last cell cannot move later")
        assertEquals(listOf("n2/p2", "n1/p1", "n3/p3", "n4/p4"), mosaic.panes)
    }

    // Reordering watches nothing new and drops nothing: the streams belong to the set of panes,
    // and the order is only how they are laid out.
    @Test
    fun reorderingChangesNoStreamsAndKeepsTheFocus() {
        val mosaic = four()
        mosaic.focus("n3/p3")
        mosaic.move("n3/p3", 0)
        assertEquals("n3/p3", mosaic.focused)
        assertEquals(4, mosaic.observers)
    }

    @Test
    fun aReorderedLayoutSavesAndRestores() {
        val prefs = MemoryPrefs()
        val connection = KamprConnection(CoroutineScope(Job()), KamprStore())
        val mosaic = MosaicState(prefs, connection)
        for (i in 1..3) mosaic.add("n$i/p$i")
        mosaic.save()
        mosaic.move("n3/p3", 0)
        assertFalse(mosaic.saved, "a reorder is a change to the layout like any other")
        mosaic.save()

        val restored = MosaicState(prefs, KamprConnection(CoroutineScope(Job()), KamprStore()))
        restored.restore()
        assertEquals(listOf("n3/p3", "n1/p1", "n2/p2"), restored.panes)
    }
}

class MosaicDragTest {
    private fun laidOut(): MosaicDrag {
        val drag = MosaicDrag()
        drag.place("a", 0f, 0f, 100f, 50f)
        drag.place("b", 100f, 0f, 200f, 50f)
        drag.place("c", 0f, 50f, 100f, 100f)
        return drag
    }

    @Test
    fun theCellUnderTheFingerIsTheOneItLandsOn() {
        val drag = laidOut()
        assertEquals("a", drag.at(10f, 10f))
        assertEquals("b", drag.at(150f, 20f))
        assertEquals("c", drag.at(50f, 90f))
        assertEquals(null, drag.at(500f, 500f), "a finger outside every cell drops nowhere")
    }

    // A cell that has left the mosaic must not keep its rectangle, or the next drag lands on a
    // pane that is no longer there.
    @Test
    fun aRemovedCellStopsBeingATarget() {
        val drag = laidOut()
        drag.forget("b")
        assertEquals(null, drag.at(150f, 20f))
    }
}
