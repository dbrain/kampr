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

fun paneTitle(pane: PaneInfo): String {
    val left = pane.label ?: pane.workspace ?: pane.id.substringAfter('/')
    val right = pane.agent ?: "bash"
    return "$left · $right"
}
