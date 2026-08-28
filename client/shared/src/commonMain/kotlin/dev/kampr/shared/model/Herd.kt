package dev.kampr.shared.model

import androidx.compose.runtime.Immutable
import dev.kampr.shared.wire.NodeInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.ServerMsg

@Immutable
data class Herd(
    val nodes: List<NodeInfo> = emptyList(),
    val panes: List<PaneInfo> = emptyList(),
    val stale: Boolean = false,
    val known: Boolean = false,
) {
    fun applyPatch(patch: ServerMsg.HerdPatch): Herd {
        val nodeById = nodes.associateByTo(LinkedHashMap()) { it.id }
        val paneById = panes.associateByTo(LinkedHashMap()) { it.id }
        for (n in patch.added.nodes + patch.changed.nodes) nodeById[n.id] = n
        for (p in patch.added.panes + patch.changed.panes) paneById[p.id] = p
        for (id in patch.removedIds) {
            nodeById.remove(id)
            paneById.remove(id)
        }
        val survivingNodes = nodeById.values.toList()
        val nodeIds = survivingNodes.map { it.id }.toSet()
        return copy(
            nodes = survivingNodes,
            panes = paneById.values.filter { it.nodeId in nodeIds || nodeIds.isEmpty() },
            stale = false,
            known = true,
        )
    }
}

@Immutable
data class NodeGroup(val node: NodeInfo, val panes: List<PaneInfo>)

enum class AgentStatus { Idle, Working, Blocked, Done, Unknown }

fun statusOf(pane: PaneInfo): AgentStatus = when (pane.agentStatus) {
    "idle" -> AgentStatus.Idle
    "working" -> AgentStatus.Working
    "blocked" -> AgentStatus.Blocked
    "done" -> AgentStatus.Done
    else -> AgentStatus.Unknown
}

fun Herd.groups(): List<NodeGroup> {
    val byNode = panes.groupBy { it.nodeId }
    val ordered = nodes.sortedByDescending { it.kind == "local" }
    return ordered.map { NodeGroup(it, byNode[it.id].orEmpty().sortedWith(paneOrder)) }
}

private val paneOrder = compareBy<PaneInfo>(
    { statusRank(statusOf(it)) },
    { it.workspace ?: "" },
    { it.id },
)

private fun statusRank(status: AgentStatus): Int = when (status) {
    AgentStatus.Blocked -> 0
    AgentStatus.Working -> 1
    AgentStatus.Done -> 2
    AgentStatus.Idle -> 3
    AgentStatus.Unknown -> 4
}

// A pane that has left the herd, and which half of it left. A shell that exits takes its pane with
// it; a node that goes takes every pane it had. Both leave the screen sitting on the last grid the
// pane printed, and the operator is owed the difference — one of them is over, the other may be
// back on its own.
//
// Only a herd that has arrived and is current can say a pane is absent. Before the first one there
// is nothing to be absent from, and a stale herd is the last one that arrived rather than a
// statement about now — reading absence out of either would report a shell closed every time the
// socket dropped, which is the reassuring-lie shape this project has paid for before.
enum class PaneGone { Shell, Node }

fun Herd.gone(paneId: String): PaneGone? = when {
    !known || stale -> null
    panes.any { it.id == paneId } -> null
    nodes.any { it.id == paneId.substringBefore('/') } -> PaneGone.Shell
    else -> PaneGone.Node
}

fun paneTitle(pane: PaneInfo): String = Naming.default.render(fieldsOf(pane))
