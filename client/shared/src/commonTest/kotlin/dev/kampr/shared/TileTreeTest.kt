package dev.kampr.shared

import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.TileNode
import dev.kampr.shared.model.groups
import dev.kampr.shared.model.leaves
import dev.kampr.shared.model.sections
import dev.kampr.shared.model.tabOf
import dev.kampr.shared.model.tileTree
import dev.kampr.shared.wire.Cut
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg
import dev.kampr.shared.wire.Tile
import dev.kampr.shared.wire.Wire
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

private const val NODE = "01JNODE"
private const val GROUP = "$NODE/k:01ja"

private fun tiled(id: String, vararg cuts: Cut, group: String = GROUP) = PaneInfo(id = "$NODE/$id", nodeId = NODE, tile = Tile(group, cuts.toList()))

private val RIGHT = "right"
private val DOWN = "down"

// The node sends each pane its path from the root of its tab's split tree (#616), and a client
// rebuilds the whole tree from those paths — the shape crates/kampr-tui/src/mosaic.rs draws.
class TileTreeTest {
    // A | (B over C), then C split right again: three levels, and the order of arrival is not the
    // order of the tree.
    private val a = tiled("n:a", Cut(RIGHT, 400))
    private val b = tiled("n:b", Cut(RIGHT, 400, second = true), Cut(DOWN, 500))
    private val c = tiled("n:c", Cut(RIGHT, 400, second = true), Cut(DOWN, 500, second = true), Cut(RIGHT, 300))
    private val d = tiled("n:d", Cut(RIGHT, 400, second = true), Cut(DOWN, 500, second = true), Cut(RIGHT, 300, second = true))

    @Test
    fun aTabsTreeIsRebuiltFromEachPanesPathWhateverOrderTheyArriveIn() {
        val tree = tileTree(listOf(d, b, a, c))
        val expected = TileNode.Split(
            down = false,
            ratio = 400,
            first = TileNode.Leaf(a.id),
            second = TileNode.Split(
                down = true,
                ratio = 500,
                first = TileNode.Leaf(b.id),
                second = TileNode.Split(false, 300, TileNode.Leaf(c.id), TileNode.Leaf(d.id)),
            ),
        )
        assertEquals(expected, tree)
        assertEquals(listOf(a.id, b.id, c.id, d.id), tree?.leaves())
    }

    // A pane that left its tab takes its box with it: the half left standing fills the split, rather
    // than the tree keeping a hole where nothing is.
    @Test
    fun aSplitMissingOneHalfIsTheHalfThatIsThere() {
        assertEquals(
            TileNode.Split(false, 400, TileNode.Leaf(a.id), TileNode.Leaf(b.id)),
            tileTree(listOf(a, b)),
        )
        assertEquals(TileNode.Leaf(a.id), tileTree(listOf(a)))
        assertNull(tileTree(listOf(PaneInfo("$NODE/w1:p1", NODE))))
    }

    @Test
    fun aPanesTabIsEveryPaneInItsGroupInTheTreesOrder() {
        val elsewhere = tiled("n:z", group = "$NODE/k:other")
        val herd = Herd(nodes = listOf(NodeInfo(NODE)), panes = listOf(d, elsewhere, b, a, c), known = true)
        assertEquals(listOf(a.id, b.id, c.id, d.id), herd.tabOf(c.id).map { it.id })
        assertEquals(listOf(elsewhere.id), herd.tabOf(elsewhere.id).map { it.id })
        assertEquals(emptyList(), herd.tabOf("$NODE/w9:p9").map { it.id })
    }

    @Test
    fun aTileArrivesOnThePaneRecord() {
        val frame = """{"t":"herd","nodes":[{"id":"$NODE"}],"panes":[
            {"id":"$NODE/n:a","node_id":"$NODE","tile":{"group":"$GROUP","path":[{"split":"right","ratio":400,"second":true},{"split":"down","ratio":500}]}}
        ]}"""
        val pane = (Wire.decode(frame) as ServerMsg.Herd).panes.single()
        assertEquals(Tile(GROUP, listOf(Cut(RIGHT, 400, second = true), Cut(DOWN, 500))), pane.tile)
    }

    // Node, then workspace, then tab. A pane in no workspace stays under its node with no heading, and
    // a workspace says which tab only when it has more than one.
    @Test
    fun theHerdIsListedByWorkspaceAndThenByTab() {
        fun p(id: String, workspace: String?, tab: String?, status: String = "idle") = PaneInfo(
            id = "$NODE/$id",
            nodeId = NODE,
            workspaceId = workspace?.let { "$NODE/kw:$it" },
            workspace = workspace,
            tabId = tab?.let { "$NODE/k:$it" },
            tab = tab,
            agentStatus = status,
            provider = "native",
        )
        val herd = Herd(
            nodes = listOf(NodeInfo(NODE, kind = "local")),
            panes = listOf(
                p("n:1", "api", "server"),
                p("n:2", null, null),
                p("n:3", "web", "ui", status = "blocked"),
                p("n:4", "api", "tests", status = "working"),
                p("n:5", "api", "server"),
            ),
            known = true,
        )
        val sections = herd.groups().single().sections
        assertEquals(
            listOf(
                null to listOf("n:2"),
                "web" to listOf("n:3"),
                "api · tests" to listOf("n:4"),
                "api · server" to listOf("n:1", "n:5"),
            ),
            sections.map { s -> s.heading to s.panes.map { it.id.substringAfter('/') } },
        )
    }
}
