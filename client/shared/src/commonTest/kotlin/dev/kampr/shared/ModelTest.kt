package dev.kampr.shared

import dev.kampr.shared.model.CellBuffer
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.model.StyleTable
import dev.kampr.shared.model.TAIL
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
        assertEquals('x'.code, cells.codePointAt(0, 0))
        assertEquals(3, cells.styleAt(1, 0))
        assertEquals(' '.code, cells.codePointAt(2, 0))
        assertEquals(0, cells.styleAt(9, 0))
    }

    @Test
    fun runsLongerThanTheRowAreClipped() {
        val cells = CellBuffer(4, 1)
        cells.apply(RowDiff(0, listOf(Run(1, "abcdefgh"))))
        assertEquals('d'.code, cells.codePointAt(3, 0))
    }

    // Probe #210: a double-width glyph occupies two columns, and the run that carries it says so
    // with `w`. The client has no Unicode width table and must not need one.
    @Test
    fun aDoubleWidthRunClaimsTwoColumnsPerGlyph() {
        val cells = CellBuffer(12, 1)
        cells.apply(RowDiff(0, listOf(Run(0, "AB"), Run(0, "\u65e5\u672c\u8a9e", w = 2), Run(0, "CD"))))
        assertEquals("AB\u65e5\u672c\u8a9eCD", cells.rowText(0))
        assertEquals('\u65e5'.code, cells.codePointAt(2, 0))
        assertEquals(TAIL, cells.codePointAt(3, 0), "column 3 is the other half of the glyph at column 2")
        assertEquals('\u672c'.code, cells.codePointAt(4, 0))
        assertEquals('C'.code, cells.codePointAt(8, 0))
        assertEquals(' '.code, cells.codePointAt(10, 0))
    }

    // A CharArray splits an astral glyph into two surrogate halves in two cells; a code point does
    // not. Same probe, second defect.
    @Test
    fun anAstralGlyphIsOneCellNotTwoSurrogateHalves() {
        val cells = CellBuffer(8, 1)
        cells.apply(RowDiff(0, listOf(Run(0, "XY"), Run(0, "\uD83D\uDE80", w = 2), Run(0, "ZW"))))
        assertEquals("XY\uD83D\uDE80ZW", cells.rowText(0))
        assertEquals(0x1F680, cells.codePointAt(2, 0))
        assertEquals(TAIL, cells.codePointAt(3, 0))
        assertEquals('Z'.code, cells.codePointAt(4, 0))
    }

    // Probe #215: a cell is a grapheme. The marks ride beside the text in `m` rather than inside
    // it, so `x` stays one code point per column and a row's width is still countable from it.
    @Test
    fun aCellWearsTheMarksItsRunDeclares() {
        val cells = CellBuffer(8, 1)
        cells.apply(RowDiff(0, listOf(Run(0, "rese", m = listOf("", "\u0301", "", "\u0301")))))
        assertEquals("re\u0301se\u0301", cells.rowText(0))
        assertEquals('e'.code, cells.codePointAt(1, 0), "the base is still one code point")
        assertEquals("\u0301", cells.marksAt(1, 0))
        assertEquals("", cells.marksAt(0, 0))
        assertEquals('s'.code, cells.codePointAt(2, 0), "and s is still in column 2")
    }

    @Test
    fun aWideClusterWearsItsJoinersAndKeepsItsTail() {
        val family = "\uD83D\uDC68\u200D\uD83D\uDC69\u200D\uD83D\uDC67"
        val cells = CellBuffer(8, 1)
        cells.apply(
            RowDiff(
                0,
                listOf(
                    Run(0, "ZZ"),
                    Run(0, "\uD83D\uDC68", w = 2, m = listOf(family.substring(2))),
                    Run(0, "XY"),
                ),
            ),
        )
        assertEquals("ZZ" + family + "XY", cells.rowText(0))
        assertEquals(TAIL, cells.codePointAt(3, 0))
        assertEquals("", cells.marksAt(3, 0), "the tail wears nothing; the lead wears it all")
        assertEquals('X'.code, cells.codePointAt(4, 0))
    }

    @Test
    fun marksAreClearedByTheNextRowThatDoesNotDeclareThem() {
        val cells = CellBuffer(4, 1)
        cells.apply(RowDiff(0, listOf(Run(0, "ee", m = listOf("\u0301", "\u0301")))))
        cells.apply(RowDiff(0, listOf(Run(0, "ee"))))
        assertEquals("ee", cells.rowText(0))
    }

    @Test
    fun aWideGlyphWithOnlyOneColumnLeftIsDropped() {
        val cells = CellBuffer(3, 1)
        cells.apply(RowDiff(0, listOf(Run(0, "ab"), Run(0, "\u65e5", w = 2))))
        assertEquals("ab", cells.rowText(0))
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

    // The node publishes `pending` on a blocked-state edge, and its first attempt at a newly
    // blocked pane carries nothing at all — the question is not readable yet. So a reconnect onto
    // a pane that is blocked again shows the previous connection's question until the retry lands,
    // and answering it lands on a pane with nothing matching to answer.
    @Test
    fun aDroppedSocketTakesEveryPanesPendingQuestionWithIt() {
        val store = KamprStore()
        store.accept(ServerMsg.Herd(listOf(NodeInfo("n1", "one", kind = "local")), listOf(pane("n1/p1", "n1", "blocked"))))
        store.accept(ServerMsg.Pending("n1/p1", "Apply this patch?", emptyList(), "screen"))
        assertEquals("Apply this patch?", store.triage().single().question)

        store.markStale()
        assertNull(
            store.triage().single().question,
            "the previous connection's question survived the socket that carried it",
        )
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
        assertEquals('h'.code, pane.cells.codePointAt(0, 0))
        assertEquals(1, pane.cells.styleAt(0, 0))

        store.accept(
            Wire.decode(
                """{"t":"grid.reset","pane":"p","cols":8,"rows":2,
                   "rows_data":[{"row":0,"runs":[{"s":1,"x":"world"}]}],
                   "cursor":{"col":5,"row":0,"visible":true},"links":[]}"""
            )!!
        )
        assertFalse(pane.stale)
        assertEquals('w'.code, pane.cells.codePointAt(0, 0))
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
