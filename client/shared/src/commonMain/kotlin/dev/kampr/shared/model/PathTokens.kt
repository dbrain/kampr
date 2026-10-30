package dev.kampr.shared.model

private const val LONGEST_PATH = 1024

private val LINE_SUFFIX = Regex("(:\\d+)+$")

private val EXTENSION = Regex("\\.[A-Za-z0-9_]*[A-Za-z][A-Za-z0-9_]*$")

private const val NOT_IN_A_PATH = "<>\"'`|*?{}$;=()[],"

private fun anchored(path: String): Boolean = path.startsWith("/") || path.startsWith("~/") ||
    path.startsWith("./") || path.startsWith("../")

// `bare` is a token in running prose, where a slash is common English (`and/or`, `1/2`) and a
// dotted word is an abbreviation or a version, so only an anchored path or a nested one ending in
// a real extension is taken. A code span is the agent saying "this is a name", so a lone filename
// with an extension is taken there as well.
fun asPath(token: String, bare: Boolean): String? {
    val path = token.replace(LINE_SUFFIX, "")
    if (path.length < 2 || path.length > LONGEST_PATH) return null
    if (path.any { it.isWhitespace() || it.code < 0x20 || it in NOT_IN_A_PATH }) return null
    if ("://" in path || path.startsWith("//") || "::" in path) return null
    val segments = path.removePrefix("/").removeSuffix("/").split('/')
    if (segments.any { it.isEmpty() }) return null
    if (segments.none { segment -> segment.any { it.isLetter() } }) return null
    val named = EXTENSION.containsMatchIn(segments.last())
    return when {
        anchored(path) -> path.takeIf { path != "./" && path != "../" }
        '/' in path -> path.takeIf { named }
        !bare -> path.takeIf { named }
        else -> null
    }
}

// What the node's route is asked for. A relative path is the pane's, so it is joined to the pane's
// directory here; the route itself refuses anything that is not absolute or `~/`-anchored.
fun resolvePath(path: String, cwd: String?): String {
    if (path.startsWith("/") || path.startsWith("~/")) return path
    val base = cwd?.takeIf { it.startsWith("/") || it.startsWith("~") } ?: "~"
    val parts = ArrayDeque(base.trimEnd('/').split('/'))
    for (segment in path.split('/')) {
        when (segment) {
            "", "." -> Unit
            ".." -> if (parts.size > 1) parts.removeLast()
            else -> parts.addLast(segment)
        }
    }
    val joined = parts.joinToString("/")
    return if (joined.isEmpty()) "/" else joined
}
