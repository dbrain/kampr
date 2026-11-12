package dev.kampr.conversation.files

import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.items
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.unit.dp
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.FsNamed
import dev.kampr.shared.net.FsNames
import dev.kampr.shared.net.decodeNames
import dev.kampr.shared.net.namesAttachmentId
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LocalPaneIo
import kotlinx.coroutines.delay

private const val SEARCH_BELOW_FROM = 2
private const val DEBOUNCE_MS = 250L

const val CANNOT_SEARCH_BELOW = "This node can't search below this folder"

data class Below(val found: FsNames? = null, val searching: Boolean = false, val refused: String? = null)

@Composable
fun rememberBelow(anchor: String, dir: String, query: String): Below {
    val io = LocalPaneIo.current
    val needle = query.trim()
    var below by remember(anchor, dir) { mutableStateOf(Below()) }
    LaunchedEffect(anchor, dir, needle) {
        if (needle.length < SEARCH_BELOW_FROM) {
            below = Below()
            return@LaunchedEffect
        }
        delay(DEBOUNCE_MS)
        below = below.copy(searching = true, refused = null)
        below = when (val got = io.attachment(anchor, namesAttachmentId(dir, needle))) {
            is AttachmentBytes.Failed -> Below(refused = if (got.missing) CANNOT_SEARCH_BELOW else got.reason)
            is AttachmentBytes.Ok -> decodeNames(got.bytes)?.let { Below(found = it) } ?: Below(refused = CANNOT_SEARCH_BELOW)
        }
    }
    return below
}

private fun relativeParent(dir: String, path: String): String {
    val parent = path.substringBeforeLast('/', "")
    val prefix = if (dir.endsWith('/')) dir else "$dir/"
    return when {
        parent == dir.trimEnd('/') -> ""
        parent.startsWith(prefix) -> parent.removePrefix(prefix)
        else -> parent
    }
}

fun LazyListScope.searchBelow(
    below: Below,
    dir: String,
    shown: Set<String>,
    onOpen: (FsNamed) -> Unit,
) {
    val found = below.found
    val hits = found?.matches.orEmpty().filter { it.path !in shown }
    if (found == null && !below.searching && below.refused == null) return
    item(key = "below-caption") {
        val tokens = Kampr.tokens
        KText("Below ${baseName(dir).ifEmpty { "/" }}", tokens.type.micro, tokens.color.mute, Modifier.padding(top = 12.dp, bottom = 2.dp))
    }
    items(hits, key = { "below ${it.path}" }) { hit ->
        val name = baseName(hit.path)
        val where = relativeParent(dir, hit.path)
        val spoken = listOfNotNull(
            if (hit.dir) "folder $name" else name,
            where.takeIf { it.isNotEmpty() }?.let { "in $it" },
        ).joinToString(", ")
        EntryRow(if (hit.dir) KamprIcons.folder else FilesIcons.file, name, where, null, spoken) { onOpen(hit) }
    }
    val note = when {
        below.refused != null -> below.refused
        below.searching -> "Searching…"
        hits.isEmpty() -> "Nothing below matches"
        found?.truncated == true -> "Showing the first ${found.matches.size}"
        else -> null
    }
    note?.let { words ->
        item(key = "below-note") {
            val tokens = Kampr.tokens
            KText(words, tokens.type.captionSmall, tokens.color.mute, Modifier.padding(vertical = 8.dp), maxLines = 2)
        }
    }
}
