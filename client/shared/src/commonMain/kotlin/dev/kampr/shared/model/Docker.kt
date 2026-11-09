package dev.kampr.shared.model

import androidx.compose.runtime.Immutable
import dev.kampr.shared.util.relativeSeconds
import dev.kampr.shared.wire.ContainerInfo
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.isMuffit

private const val DAY_SECONDS = 86_400L

const val ADOPTION_FAILING = "adoption failing"

private val ALIVE = setOf("running", "restarting", "paused")

fun ContainerInfo.wanted(nowSeconds: Long): Boolean = state in ALIVE ||
    health == "unhealthy" ||
    adoptError != null ||
    (lastCrash?.let { it.at >= nowSeconds - DAY_SECONDS } == true)

fun NodeGroup.shellsOf(container: ContainerInfo): List<PaneInfo> = panes.filter { it.container == container.name && !it.isMuffit }

fun NodeGroup.childOf(container: ContainerInfo): NodeGroup? = container.child?.let { id -> children.firstOrNull { it.node.id == id } }

// An adopted node whose container has no row to sit under: the container went, or the host is too
// old to say which one it is.
val NodeGroup.strayChildren: List<NodeGroup>
    get() = children.filter { child -> node.containers.none { it.child == child.node.id } }

// A container the operator has a shell in or a node inside is shown whatever its state: hiding it
// would hide the panes that hang off it.
private fun NodeGroup.pinned(container: ContainerInfo): Boolean = shellsOf(container).isNotEmpty() || childOf(container) != null

@Immutable
data class Shelf(val shown: List<ContainerInfo>, val stopped: List<ContainerInfo>)

fun NodeGroup.shelf(nowSeconds: Long): Shelf {
    val (shown, stopped) = node.containers.partition { it.wanted(nowSeconds) || pinned(it) }
    return Shelf(shown, stopped)
}

@Immutable
data class ProjectGroup(val project: String?, val containers: List<ContainerInfo>)

fun List<ContainerInfo>.byProject(): List<ProjectGroup> = groupBy { it.project }
    .map { (project, members) -> ProjectGroup(project, members.sortedBy { it.name }) }
    .sortedWith(compareBy({ it.project != null }, { it.project.orEmpty() }))

fun containerPhrase(container: ContainerInfo, nowMillis: Double): String = when (container.state) {
    "running" -> container.startedAt?.let { "up ${relativeSeconds(it, nowMillis).let { e -> if (e == "now") "<1m" else e }}" } ?: "running"

    "exited", "dead" -> {
        val how = listOfNotNull(
            listOfNotNull(container.state, container.exitCode?.toString()).joinToString(" "),
            "OOM".takeIf { container.oom },
        ).joinToString(" · ")
        container.finishedAt?.let { "$how ${ago(it, nowMillis)}" } ?: how
    }

    "missing" -> "gone"

    else -> container.state.ifEmpty { "unknown" }
}

fun ago(atSeconds: Long, nowMillis: Double): String = relativeSeconds(atSeconds, nowMillis).let { if (it == "now") "just now" else "$it ago" }

fun containerSpoken(container: ContainerInfo, nowMillis: Double): String {
    val phrase = containerPhrase(container, nowMillis)
    return listOfNotNull(
        "Container ${container.name}",
        container.state.takeUnless { phrase.startsWith(it) || it == "missing" },
        container.health,
        phrase,
        container.restarts.takeIf { it > 0 }?.let { "restarted $it ${if (it == 1) "time" else "times"}" },
        ADOPTION_FAILING.takeIf { container.adoptError != null },
    ).joinToString(", ")
}

fun formatBytes(bytes: Long): String {
    val mib = bytes / 1_048_576.0
    return when {
        mib >= 1024 -> "${(mib / 1024 * 10).toLong() / 10.0} GB"
        mib >= 1 -> "${mib.toLong()} MB"
        else -> "${bytes / 1024} KB"
    }
}

val Herd.anyAdoptionFailing: Boolean get() = nodes.any { node -> node.containers.any { it.adoptError != null } }
