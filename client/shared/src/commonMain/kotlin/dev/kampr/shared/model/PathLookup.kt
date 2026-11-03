package dev.kampr.shared.model

import androidx.compose.runtime.Immutable

// What a tapped relative path was before it was joined to the pane's directory, and the
// directories the reply named before it, nearest first. Enough to look for the file when the
// join was the wrong guess.
@Immutable
data class Lookup(val tail: String, val cwd: String?, val near: List<String>)

private const val MOST_NEAR = 6

private val EXTENSION = Regex("\\.[A-Za-z0-9_]*[A-Za-z][A-Za-z0-9_]*$")

private const val DRESSING = "([{\"'<*"
private const val TRAILING = ".,;:!?)]}\"'>*"

private fun anchored(path: String): Boolean = path.startsWith("/") || path.startsWith("~")

fun lookupFor(path: String, cwd: String?, near: List<String> = emptyList()): Lookup? {
    if (anchored(path)) return null
    return Lookup(path.removePrefix("./"), cwd, near.take(MOST_NEAR))
}

// A name with an extension is a file, and a reply that says "look in /etc/hosts" was not naming a
// directory to read the next path against.
private fun dirMention(path: String): String? {
    if (!anchored(path)) return null
    val dir = path.trimEnd('/')
    if (dir.isEmpty() || dir == "~") return null
    return dir.takeIf { path.endsWith('/') || !EXTENSION.containsMatchIn(dir.substringAfterLast('/')) }
}

fun mentionedDirs(text: String): List<String> = text
    .split(' ', '\t', '\n', '\r', '`', '|')
    .map { it.trimStart(*DRESSING.toCharArray()).trimEnd(*TRAILING.toCharArray()) }
    .mapNotNull { token -> asPath(token, bare = false)?.let(::dirMention) }

fun nearer(mentioned: List<String>, before: List<String>): List<String> = (mentioned.asReversed() + before).distinct().take(MOST_NEAR)

// The one decision about which of several files a path meant: the shallowest, then the first by
// name, so the same tap always opens the same file.
fun bestMatch(found: Collection<String>): String? = found.minWithOrNull(compareBy<String>({ it.count { c -> c == '/' } }, { it }))
