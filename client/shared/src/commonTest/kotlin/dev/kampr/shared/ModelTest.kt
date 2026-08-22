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
    fun scrollbackKeepsWhatItHoldsWhenATailContinuesFromTheKnownEnd() {
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
                """{"t":"scrollback","pane":"p","from_top":2,
                   "rows":[{"row":2,"runs":[{"s":0,"x":"new"}]}],
                   "total_rows":1,"complete":true,"capped":true}"""
            )!!
        )
        val scrollback = store.pane("p").scrollback
        assertEquals(0, scrollback.fromTop)
        assertEquals(3, scrollback.historyRows)
        assertEquals("old", scrollback.row(0)?.runs?.first()?.x)
        assertEquals("new", scrollback.row(2)?.runs?.first()?.x)
        assertTrue(scrollback.capped)
    }

    @Test
    fun scrollbackDropsRowsWhenTheRingRestartsBeyondWhatItHolds() {
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
                """{"t":"scrollback","pane":"p","from_top":9,
                   "rows":[{"row":9,"runs":[{"s":0,"x":"new"}]}],
                   "total_rows":1,"complete":true,"capped":true}"""
            )!!
        )
        val scrollback = store.pane("p").scrollback
        assertEquals(9, scrollback.fromTop)
        assertNull(scrollback.row(0))
        assertEquals("new", scrollback.row(9)?.runs?.first()?.x)
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
    // The Claude transcript in crates/kampr-journal/tests/fixtures carries a resumed session, so
    // its last record is stamped three weeks BEFORE the ones above it. Transcript order is the
    // only order a client may show; sorting on `at` reorders a real conversation.
    @Test
    fun convoKeepsTheNodesTurnOrderEvenWhenTimestampsGoBackwards() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"convo","pane":"p","cursor":"t1","more":true,"turns":[
                   {"id":"t1","role":"user","at":"2026-08-17T03:47:14Z","blocks":[{"b":"md","text":"a"}]},
                   {"id":"t2","role":"assistant","at":"2026-08-17T03:48:40Z","blocks":[{"b":"md","text":"b"}]},
                   {"id":"t3","role":"assistant","at":"2026-07-30T00:52:59Z","blocks":[{"b":"md","text":"c"}]}]}"""
            )!!
        )
        assertEquals(listOf("t1", "t2", "t3"), store.pane("p").turns.map { it.id })
    }

    @Test
    fun convoLoadPrependsTheOlderPageAndAdvancesTheCursor() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"convo","pane":"p","cursor":"t3","more":true,"turns":[
                   {"id":"t3","role":"user","blocks":[{"b":"md","text":"c"}]},
                   {"id":"t4","role":"assistant","blocks":[{"b":"md","text":"d"}]}]}"""
            )!!
        )
        store.accept(
            Wire.decode(
                """{"t":"convo","pane":"p","cursor":"t1","more":false,"turns":[
                   {"id":"t1","role":"user","blocks":[{"b":"md","text":"a"}]},
                   {"id":"t2","role":"assistant","blocks":[{"b":"md","text":"b"}]}]}"""
            )!!
        )
        val pane = store.pane("p")
        assertEquals(listOf("t1", "t2", "t3", "t4"), pane.turns.map { it.id })
        assertEquals("t1", pane.convoCursor)
        assertFalse(pane.convoMore)
    }

    // kampr-journal revises a tool turn in place when its result lands, so a `convo.turn` that
    // repeats an id is a replacement. Appending it instead duplicates every tool call.
    @Test
    fun convoTurnRevisesARunningToolInPlaceRatherThanAppending() {
        val store = KamprStore()
        store.accept(
            Wire.decode(
                """{"t":"convo","pane":"p","cursor":"t1","more":false,"turns":[
                   {"id":"t1","role":"assistant","blocks":[
                     {"b":"tool","name":"Edit","summary":"surface.rs","state":"running"}]}]}"""
            )!!
        )
        store.accept(
            Wire.decode(
                """{"t":"convo.turn","pane":"p","turns":[
                   {"id":"t1","role":"assistant","blocks":[
                     {"b":"tool","name":"Edit","summary":"surface.rs","lines":3,"state":"done"},
                     {"b":"diff","path":"surface.rs","text":"@@ -1,1 +1,1 @@\n-a\n+b\n"}]},
                   {"id":"t2","role":"assistant","blocks":[{"b":"md","text":"done"}]}]}"""
            )!!
        )
        val turns = store.pane("p").turns
        assertEquals(listOf("t1", "t2"), turns.map { it.id })
        assertEquals("done", (turns[0].blocks[0] as dev.kampr.shared.wire.Block.Tool).state)
        assertEquals(2, turns[0].blocks.size)
    }
}
