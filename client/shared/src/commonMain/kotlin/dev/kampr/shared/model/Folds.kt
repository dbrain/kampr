package dev.kampr.shared.model

import androidx.compose.runtime.Immutable

enum class Fold(val key: String, val openByDefault: Boolean) {
    Muffit("muffit", true),
    Containers("containers", false),
    Stopped("stopped", false),
}

// Per device and per (node, section): which of a host's sections the operator has opened or shut.
@Immutable
data class Folds(private val set: Map<String, Boolean> = emptyMap()) {
    fun open(node: String, fold: Fold): Boolean = set[keyOf(node, fold)] ?: fold.openByDefault

    fun with(node: String, fold: Fold, open: Boolean): Folds = copy(set = set + (keyOf(node, fold) to open))

    fun encode(): String = set.entries.joinToString("\n") { (key, open) -> "$key\t${if (open) 1 else 0}" }

    companion object {
        // A node id never holds a tab, so it separates the three fields.
        private fun keyOf(node: String, fold: Fold) = "$node\t${fold.key}"

        fun decode(text: String?): Folds = Folds(
            text.orEmpty().lines().mapNotNull { line ->
                val parts = line.split('\t')
                if (parts.size != 3 || Fold.entries.none { it.key == parts[1] }) return@mapNotNull null
                "${parts[0]}\t${parts[1]}" to (parts[2] == "1")
            }.toMap(),
        )
    }
}
