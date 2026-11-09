package dev.kampr.conversation.files

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.IconGlyph
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.KamprIcons
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.announce

@Composable
fun ArchiveView(listing: ZipListing.Ok, modifier: Modifier = Modifier) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val summary = remember(listing) { archiveSummary(listing) }
    LazyColumn(
        modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = 12.dp + safe.left, end = 12.dp + safe.right, top = 4.dp, bottom = 16.dp),
    ) {
        item(key = "summary") {
            KText(summary, tokens.type.captionSmall, tokens.color.mute, Modifier.padding(vertical = 6.dp).announce(summary), maxLines = 3)
        }
        itemsIndexed(listing.entries, key = { at, _ -> at }) { _, entry -> ArchiveRow(entry) }
    }
}

@Composable
private fun ArchiveRow(entry: ZipEntry) {
    val tokens = Kampr.tokens
    val facts = listOfNotNull(
        formatBytes(entry.size).takeIf { !entry.isDir },
        entry.modified,
        "encrypted".takeIf { entry.encrypted },
    ).joinToString(" · ")
    Row(
        Modifier.fillMaxWidth().semantics(mergeDescendants = true) {}.padding(horizontal = 4.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        IconGlyph(if (entry.isDir) KamprIcons.folder else FilesIcons.file, 16.dp, tokens.color.dim)
        Column(Modifier.weight(1f)) {
            KText(entry.path, tokens.type.caption, if (entry.isDir) tokens.color.dim else tokens.color.text)
            if (facts.isNotEmpty()) KText(facts, tokens.type.micro, tokens.color.mute)
        }
    }
}

private fun archiveSummary(listing: ZipListing.Ok): String {
    val files = listing.entries.count { !it.isDir }
    val folders = listing.entries.size - files
    val unpacked = listing.entries.sumOf { if (it.isDir) 0L else it.size }
    val counts = listOfNotNull(
        plural(files, "file"),
        plural(folders, "folder").takeIf { folders > 0 },
    ).joinToString(", ")
    val head = if (listing.entries.isEmpty()) "An empty archive." else "$counts, ${formatBytes(unpacked)} uncompressed."
    if (listing.entries.size >= listing.total) return head
    return "$head Showing the first ${grouped(listing.entries.size.toLong())} of ${grouped(listing.total)} entries."
}

private fun plural(n: Int, word: String) = "${grouped(n.toLong())} $word${if (n == 1) "" else "s"}"

private fun grouped(n: Long): String = n.toString().reversed().chunked(3).joinToString(",").reversed()
