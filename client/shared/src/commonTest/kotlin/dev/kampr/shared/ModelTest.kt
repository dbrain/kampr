package dev.kampr.shared

import dev.kampr.shared.model.CellBuffer
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.groups
import dev.kampr.shared.wire.HerdDelta
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Run
import dev.kampr.shared.wire.RowDiff
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Style
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFalse
import kotlin.test.assertNull
import kotlin.test.assertTrue

private fun pane(id: String, node: String, status: String = "idle") =
    PaneInfo(id = id, nodeId = node, workspace = "w", cwd = "~/x", agentStatus = status)

class ModelTest {
    @Test
    fun rowsPadWithBlanksAndTrailingCellsAreCleared() {
        val cells = CellBuffer(10, 2)
        cells.apply(RowDiff(0, listOf(Run(0, "abcdefghij"))))
        cells.apply(RowDiff(0, listOf(Run(3, "xy"))))
        assertEquals('x', cells.charAt(0, 0))
        assertEquals(3, cells.styleAt(1, 0))
        assertEquals(' ', cells.charAt(2, 0))
        assertEquals(0, cells.styleAt(9, 0))
    }

    @Test
    fun runsLongerThanTheRowAreClipped() {
        val cells = CellBuffer(4, 1)
        cells.apply(RowDiff(0, listOf(Run(1, "abcdefgh"))))
        assertEquals('d', cells.charAt(3, 0))
    }

    @Test
    fun linkIdsRoundTripThroughTheBuffer() {
        val cells = CellBuffer(6, 1)
        cells.apply(RowDiff(0, listOf(Run(0, "ab"), Run(0, "cd", l = 0), Run(0, "ef"))))
        assertEquals(-1, cells.linkAt(0, 0))
        assertEquals(0, cells.linkAt(2, 0))
        assertEquals(-1, cells.linkAt(4, 0))
    }

    @Test
    fun styleTableIsAppendOnlyAndStableById() {
        val table = StyleTable()
        table.append(1, listOf(Style(bold = true), Style(italic = true)))
        table.append(3, listOf(Style(underline = true)))
        assertTrue(table[1].bold)
        assertTrue(table[2].italic)
        assertTrue(table[3].underline)
        assertFalse(table[0].bold)
        assertFalse(table[99].bold)
    }

    @Test
    fun herdPatchAddsChangesAndRemoves() {
        val base = Herd(
            nodes = listOf(NodeInfo("n1", "one", kind = "local"), NodeInfo("n2", "two")),
            panes = listOf(pane("n1/p1", "n1"), pane("n2/p1", "n2")),
            known = true,
        )
        val patched = base.applyPatch(
            ServerMsg.HerdPatch(
                added = HerdDelta(panes = listOf(pane("n1/p2", "n1", "working"))),
                changed = HerdDelta(panes = listOf(pane("n1/p1", "n1", "blocked"))),
                removedIds = listOf("n2"),
            )
        )
        assertEquals(listOf("n1"), patched.nodes.map { it.id })
        assertEquals(setOf("n1/p1", "n1/p2"), patched.panes.map { it.id }.toSet())
        assertEquals("blocked", patched.panes.first { it.id == "n1/p1" }.agentStatus)
        assertFalse(patched.stale)
    }

    @Test
    fun groupsPutTheLocalNodeFirstAndBlockedPanesOnTop() {
        val herd = Herd(
            nodes = listOf(NodeInfo("n2", "peer"), NodeInfo("n1", "hub", kind = "local")),
            panes = listOf(
                pane("n1/p1", "n1", "idle"),
                pane("n1/p2", "n1", "blocked"),
                pane("n2/p1", "n2", "working"),
            ),
            known = true,
        )
        val groups = herd.groups()
        assertEquals("hub", groups[0].node.name)
        assertEquals("n1/p2", groups[0].panes[0].id)
    }

    @Test
    fun cachedGridSurvivesADropAndIsMarkedStaleUntilTheNextReset() {
        val store = KamprStore()
        store.accept(Wire.decode("""{"t":"styles","from":1,"styles":[{"fg":{"k":"i","v":2}}]}""")!!)
        store.accept(
            Wire.decode(
                """{"t":"grid.reset","pane":"p","cols":8,"rows":2,
                   "rows_data":[{"row":0,"runs":[{"s":1,"x":"hello"}]}],
                   "cursor":{"col":5,"row":0,"visible":true},"links":[]}"""
            )!!
        )
        val pane = store.pane("p")
        assertTrue(pane.painted)
        assertFalse(pane.stale)

        store.markStale()
        assertTrue(pane.stale)
        assertEquals('h', pane.cells.charAt(0, 0))
        assertEquals(1, pane.cells.styleAt(0, 0))

        store.accept(
            Wire.decode(
                """{"t":"grid.reset","pane":"p","cols":8,"rows":2,
                   "rows_data":[{"row":0,"runs":[{"s":1,"x":"world"}]}],
                   "cursor":{"col":5,"row":0,"visible":true},"links":[]}"""
            )!!
        )
        assertFalse(pane.stale)
        assertEquals('w', pane.cells.charAt(0, 0))
    }

    @Test
    fun scrollbackDropsRowsBelowAnAdvancedFromTop() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"scrollback","pane":"p","from_top":0,
                   "rows":[{"row":0,"runs":[{"s":0,"x":"old"}]},{"row":1,"runs":[{"s":0,"x":"mid"}]}],
                   "total_rows":2,"complete":true,"capped":false}"""
            )!!
        )
        store.accept(
            Wire.decode(
                """{"t":"scrollback","pane":"p","from_top":1,
                   "rows":[{"row":1,"runs":[{"s":0,"x":"mid"}]},{"row":2,"runs":[{"s":0,"x":"new"}]}],
                   "total_rows":2,"complete":true,"capped":true}"""
            )!!
        )
        val scrollback = store.pane("p").scrollback
        assertNull(scrollback.row(0))
        assertEquals("mid", scrollback.row(1)?.runs?.first()?.x)
        assertTrue(scrollback.capped)
    }

    @Test
    fun historyDepthIsTheRowCountNotTheHighestIndex() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"scrollback","pane":"p","from_top":1493,
                   "rows":[{"row":1493,"runs":[{"s":0,"x":"npm run build"}]}],
                   "total_rows":60,"complete":true,"capped":true}"""
            )!!
        )
        val scrollback = store.pane("p").scrollback
        assertEquals(60, scrollback.historyRows)
        assertEquals(1493, scrollback.fromTop)
        assertEquals("npm run build", scrollback.row(1493)?.runs?.first()?.x)
    }
}
