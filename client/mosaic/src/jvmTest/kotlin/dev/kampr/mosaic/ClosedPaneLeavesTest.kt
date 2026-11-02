package dev.kampr.mosaic

import dev.kampr.shared.model.Herd
import dev.kampr.shared.platform.MemoryPrefs
import kotlin.test.Test
import kotlin.test.assertEquals

private val LATE = "$HUB/w9:p1"

private fun Herd.without(vararg ids: String) = copy(panes = panes.filterNot { it.id in ids })

private fun Herd.withoutNode(id: String) = copy(nodes = nodes.filterNot { it.id == id }, panes = panes.filterNot { it.nodeId == id })

private fun Herd.herdrDown(id: String) = copy(
    nodes = nodes.map { if (it.id == id) it.copy(online = false) else it },
    panes = panes.filterNot { it.nodeId == id },
)

// A shell that exits takes its cell with it; a machine that left may be back with the pane still on
// it, and a pane opened ahead of the herd that carries it has not closed, it has not arrived.
class ClosedPaneLeavesTest {
    private val fixture = Fixture().apply { fourPanes() }
    private val herd = fixture.store.herd.value

    private fun settled(vararg herds: Herd): List<String> {
        fixture.mosaic.reconcile(herd)
        for (next in herds) fixture.mosaic.reconcile(next)
        return fixture.mosaic.panes
    }

    @Test
    fun aShellThatExitsLeavesTheMosaic() = assertEquals(listOf(CLAUDE, CODEX, DOGE), settled(herd.without(SUNGROW)))

    @Test
    fun aPaneWhoseNodeLeftKeepsItsCell() = assertEquals(listOf(CLAUDE, CODEX, SUNGROW, DOGE), settled(herd.withoutNode(SUN)))

    // A herdr that is down takes its panes out of the herd and leaves the node in it, which reads
    // exactly like every shell on it exiting at once.
    @Test
    fun aHerdrThatWentDownIsNotEveryShellClosing() = assertEquals(listOf(CLAUDE, CODEX, SUNGROW, DOGE), settled(herd.herdrDown(SUN)))

    @Test
    fun aStaleHerdSaysNothingIsGone() = assertEquals(listOf(CLAUDE, CODEX, SUNGROW, DOGE), settled(herd.without(SUNGROW).copy(stale = true)))

    @Test
    fun aPaneStillArrivingIsNotClosed() {
        fixture.mosaic.remove(DOGE)
        fixture.mosaic.add(LATE)
        assertEquals(listOf(CLAUDE, CODEX, SUNGROW, LATE), settled(herd, herd))
        val arrived = herd.copy(panes = herd.panes + herd.panes.first().copy(id = LATE))
        assertEquals(listOf(CLAUDE, CODEX, SUNGROW), settled(arrived, herd))
    }

    @Test
    fun theFocusMovesToTheNeighbourOfTheCellThatClosed() {
        for ((closing, next) in listOf(SUNGROW to DOGE, DOGE to SUNGROW, CLAUDE to CODEX)) {
            val fixture = Fixture().apply { fourPanes() }
            fixture.mosaic.focus(closing)
            fixture.mosaic.reconcile(herd)
            fixture.mosaic.reconcile(herd.without(closing))
            assertEquals(next, fixture.mosaic.focused, "closing $closing")
        }
    }

    @Test
    fun aSavedLayoutDoesNotBringAClosedPaneBack() {
        val prefs = MemoryPrefs()
        val mosaic = MosaicState(prefs, fixture.connection).apply {
            restore()
            for (id in listOf(CLAUDE, SUNGROW, DOGE)) add(id)
            save()
            reconcile(herd)
            reconcile(herd.without(SUNGROW))
        }
        assertEquals(true, mosaic.saved, "the layout on disk lost the pane the screen did")
        val next = MosaicState(prefs, fixture.connection).apply { restore() }
        assertEquals(listOf(CLAUDE, DOGE), next.panes)
    }

    // The layout was made from panes in a herd, so a pane in it that is missing from a current herd
    // whose node is up closed while nobody was looking.
    @Test
    fun aPaneThatClosedWhileTheAppWasAwayIsDroppedOnRestore() {
        val prefs = MemoryPrefs().apply { set("mosaic.panes", encodeArrangement(listOf(CLAUDE, SUNGROW, DOGE))) }
        val mosaic = MosaicState(prefs, fixture.connection).apply {
            restore()
            reconcile(herd.without(SUNGROW).withoutNode(NAS))
        }
        assertEquals(listOf(CLAUDE, DOGE), mosaic.panes)
    }
}
