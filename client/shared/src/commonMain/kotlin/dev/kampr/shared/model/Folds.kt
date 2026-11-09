package dev.kampr.shared.model

import androidx.compose.runtime.Immutable

enum class Fold(val key: String, val openByDefault: Boolean) {
    Stopped("stopped", false),
}

// Which of a host's lists is showing. Terminals is always there; the other two only on a host that
// has something in them.
enum class Area(val key: String) {
    Terminals("terminals"),
    Containers("docker"),
    Muffit("muffit"),
}

fun NodeGroup.areas(): List<Area> = listOfNotNull(
    Area.Terminals,
    Area.Containers.takeIf { node.containers.isNotEmpty() || node.docker == false },
    Area.Muffit.takeIf { background.isNotEmpty() },
)

// Per device and per node: what the operator has opened, and which list they last picked.
@Immutable
data class Folds(private val set: Map<String, Boolean> = emptyMap(), private val picked: Map<String, Area> = emptyMap()) {
    fun open(node: String, fold: Fold): Boolean = set[keyOf(node, fold.key)] ?: fold.openByDefault

    fun with(node: String, fold: Fold, open: Boolean): Folds = copy(set = set + (keyOf(node, fold.key) to open))

    fun area(group: NodeGroup): Area = picked[group.node.id]?.takeIf { it in group.areas() } ?: Area.Terminals

    fun showing(node: String, area: Area): Folds = copy(picked = picked + (node to area))

    fun encode(): String = (
        set.entries.map { (key, open) -> "$key\t${if (open) 1 else 0}" } +
            picked.entries.map { (node, area) -> "${keyOf(node, AREA)}\t${area.key}" }
        ).joinToString("\n")

    companion object {
        private const val AREA = "area"

        // A node id never holds a tab, so it separates the three fields.
        private fun keyOf(node: String, what: String) = "$node\t$what"

        fun decode(text: String?): Folds {
            val lines = text.orEmpty().lines().map { it.split('\t') }.filter { it.size == 3 }
            return Folds(
                lines.filter { parts -> Fold.entries.any { it.key == parts[1] } }
                    .associate { keyOf(it[0], it[1]) to (it[2] == "1") },
                lines.filter { it[1] == AREA }
                    .mapNotNull { parts -> Area.entries.firstOrNull { it.key == parts[2] }?.let { parts[0] to it } }
                    .toMap(),
            )
        }
    }
}
