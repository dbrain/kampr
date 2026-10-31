package dev.kampr.shared.model

import androidx.compose.runtime.Immutable
import dev.kampr.shared.wire.PaneInfo

@Immutable
sealed interface TileNode {
    @Immutable
    data class Leaf(val pane: String) : TileNode

    // `ratio` is the first half's share in thousandths; `down` puts the second half below rather
    // than beside it.
    @Immutable
    data class Split(val down: Boolean, val ratio: Int, val first: TileNode, val second: TileNode) : TileNode
}

const val TILE_WHOLE = 1000

private class Building {
    var down = false
    var ratio = TILE_WHOLE / 2
    var cut = false
    var pane: String? = null
    var first: Building? = null
    var second: Building? = null

    fun built(): TileNode? {
        val a = first?.built()
        val b = second?.built()
        return when {
            a != null && b != null -> TileNode.Split(down, ratio, a, b)
            a != null -> a
            b != null -> b
            else -> pane?.let(TileNode::Leaf)
        }
    }
}

// The node sends each pane only its own path from the root of its tab's tree, so a patch that moves
// one divider carries only the panes it moved; the tree is every path laid over the others. Panes
// with no tile are left out, and a split whose other half has gone is the half still there.
fun tileTree(panes: List<PaneInfo>): TileNode? {
    val root = Building()
    var any = false
    for (pane in panes) {
        val tile = pane.tile ?: continue
        any = true
        var at = root
        for (cut in tile.path) {
            if (!at.cut) {
                at.cut = true
                at.down = cut.split == "down"
                at.ratio = cut.ratio.coerceIn(1, TILE_WHOLE - 1)
            }
            at = if (cut.second) {
                at.second ?: Building().also { at.second = it }
            } else {
                at.first ?: Building().also { at.first = it }
            }
        }
        at.pane = pane.id
    }
    return if (any) root.built() else null
}

fun TileNode.leaves(): List<String> = when (this) {
    is TileNode.Leaf -> listOf(pane)
    is TileNode.Split -> first.leaves() + second.leaves()
}

// Every pane in the tab `paneId` is in, in the tree's reading order; empty for a pane the node has
// placed in no tab.
fun Herd.tabOf(paneId: String): List<PaneInfo> {
    val group = panes.firstOrNull { it.id == paneId }?.tile?.group ?: return emptyList()
    val members = panes.filter { it.tile?.group == group }
    val order = tileTree(members)?.leaves().orEmpty()
    return order.mapNotNull { id -> members.firstOrNull { it.id == id } }
}

@Immutable
data class PaneSection(val heading: String?, val panes: List<PaneInfo>)

// Node, then workspace, then tab. The panes in no workspace come first with no heading, as the
// whole of a node used to; the rest are ordered by the most urgent pane in each, the same rank a
// flat list used, so a blocked agent still rises to the top of its node.
val NodeGroup.sections: List<PaneSection>
    get() {
        val loose = terminals.filter { it.workspaceKey == null }
        val named = terminals.filter { it.workspaceKey != null }.groupBy { it.workspaceKey!! }
        val out = mutableListOf<PaneSection>()
        if (loose.isNotEmpty()) out += PaneSection(null, loose)
        val workspaces = named.values.sortedWith(compareBy({ urgency(it) }, { it.first().workspace.orEmpty() }))
        for (inWorkspace in workspaces) {
            val name = inWorkspace.first().workspace ?: inWorkspace.first().workspaceKey!!.substringAfter('/')
            val tabs = inWorkspace.groupBy { it.tabId ?: it.tab }.values.sortedWith(compareBy({ urgency(it) }, { it.first().tab.orEmpty() }))
            if (tabs.size == 1) {
                out += PaneSection(name, inWorkspace)
            } else {
                for (tab in tabs) out += PaneSection(listOfNotNull(name, tab.first().tab).joinToString(" · "), tab)
            }
        }
        return out
    }

private fun urgency(panes: List<PaneInfo>): Int = panes.minOf { statusRank(statusOf(it)) }
