package dev.kampr.conversation.files

import dev.kampr.shared.net.FsEntry
import dev.kampr.shared.net.RepoChange

data class Crumb(val label: String, val path: String)

fun childPath(dir: String, name: String): String = if (dir.endsWith('/')) "$dir$name" else "$dir/$name"

fun baseName(path: String): String = path.trimEnd('/').substringAfterLast('/').ifEmpty { path }

fun crumbs(path: String, home: String?): List<Crumb> {
    val anchored = home?.takeIf { it.length > 1 && (path == it || path.startsWith("$it/")) }
    val (first, rest) = if (anchored != null) {
        Crumb("~", anchored) to path.removePrefix(anchored)
    } else {
        Crumb("/", "/") to path
    }
    var at = first.path
    return listOf(first) + rest.split('/').filter { it.isNotEmpty() }.map { part ->
        at = childPath(at, part)
        Crumb(part, at)
    }
}

// A filter that starts with a dot is asking for a dotfile, so it finds one with the toggle off.
fun visibleEntries(entries: List<FsEntry>, query: String, showHidden: Boolean): List<FsEntry> {
    val needle = query.trim().lowercase()
    val hidden = showHidden || needle.startsWith('.')
    return entries
        .filter { hidden || !it.name.startsWith('.') }
        .filter { needle.isEmpty() || it.name.lowercase().contains(needle) }
        .sortedWith(compareBy<FsEntry> { !it.isDir }.thenBy { it.name.lowercase() })
}

fun formatBytes(bytes: Long): String {
    if (bytes < 1024) return "$bytes B"
    val units = listOf("KB", "MB", "GB", "TB")
    var value = bytes.toDouble() / 1024
    var unit = 0
    while (value >= 1024 && unit < units.lastIndex) {
        value /= 1024
        unit++
    }
    val tenths = (value * 10).toLong()
    return if (value >= 10) "${value.toLong()} ${units[unit]}" else "${tenths / 10}.${tenths % 10} ${units[unit]}"
}

fun gitWord(letter: String): String = when (letter) {
    "M" -> "modified"
    "A" -> "added"
    "D" -> "deleted"
    "R" -> "renamed"
    "C" -> "copied"
    "U" -> "conflicted"
    "?" -> "untracked"
    else -> "changed"
}

data class Changed(val change: RepoChange, val letter: String)

data class ChangeGroups(val staged: List<Changed>, val unstaged: List<Changed>, val untracked: List<Changed>)

// Porcelain v1: X is the index, Y the work tree, `??` untracked. A file staged and then edited
// again is in both of the first two lists, which is what git itself would say.
fun groupChanges(changes: List<RepoChange>): ChangeGroups {
    val untracked = changes.filter { it.staged == "?" }
    val tracked = changes - untracked.toSet()
    return ChangeGroups(
        staged = tracked.filter { it.staged.isMarked() }.map { Changed(it, it.staged) },
        unstaged = tracked.filter { it.unstaged.isMarked() }.map { Changed(it, it.unstaged) },
        untracked = untracked.map { Changed(it, "?") },
    )
}

private fun String.isMarked(): Boolean = isNotBlank() && this != "?" && this != "!"

data class FilePatch(val path: String, val text: String)

data class CommitParts(val header: String, val files: List<FilePatch>)

fun splitCommit(text: String): CommitParts {
    val lines = text.split('\n')
    val starts = lines.indices.filter { lines[it].startsWith("diff --git ") }
    if (starts.isEmpty()) return CommitParts(text.trimEnd(), emptyList())
    val header = lines.subList(0, starts.first()).joinToString("\n").trimEnd()
    val files = starts.mapIndexed { i, start ->
        val end = starts.getOrElse(i + 1) { lines.size }
        filePatch(lines.subList(start, end))
    }
    return CommitParts(header, files)
}

private fun filePatch(section: List<String>): FilePatch {
    val plus = section.firstOrNull { it.startsWith("+++ ") }?.removePrefix("+++ ")
    val minus = section.firstOrNull { it.startsWith("--- ") }?.removePrefix("--- ")
    val path = sequenceOf(plus, minus)
        .filterNotNull()
        .firstOrNull { it != "/dev/null" }
        ?.removePrefix("b/")
        ?.removePrefix("a/")
        ?: section.first().substringAfter(" b/", section.first().removePrefix("diff --git "))
    val bodyFrom = section.indexOfFirst { it.startsWith("@@") }.takeIf { it >= 0 } ?: 1
    val body = section.subList(bodyFrom, section.size).joinToString("\n").trimEnd()
    return FilePatch(path, body)
}
