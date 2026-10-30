package dev.kampr.conversation.files

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import dev.kampr.conversation.detailOf
import dev.kampr.conversation.fileName
import dev.kampr.conversation.headlineOf
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.AttachmentSlice
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.lsAttachmentId
import dev.kampr.shared.net.pathOfAttachmentId
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.FilesNav
import dev.kampr.shared.ui.FilesPlace
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.ui.announce
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

internal class Source(
    val nodeId: String,
    val anchor: String,
    val id: String,
    val name: String,
    val path: String?,
    val detail: String?,
    val record: Boolean,
    val saveAs: String?,
)

private sealed interface Fetch {
    data object Asking : Fetch
    class Got(val opened: Opened) : Fetch
    data class Failed(val reason: String) : Fetch
}

@Composable
fun FileView(place: FilesPlace.File, nav: FilesNav, modifier: Modifier = Modifier) {
    val source = remember(place) {
        Source(place.nodeId, place.anchor, fileAttachmentId(place.path), fileName(place.path), place.path, place.path, record = false, saveAs = fileName(place.path))
    }
    FileScreen(source, nav, modifier)
}

@Composable
fun RecordView(place: FilesPlace.Record, nav: FilesNav, modifier: Modifier = Modifier) {
    val source = remember(place) {
        val path = pathOfAttachmentId(place.att.id)?.takeIf { fileAttachmentId(it) == place.att.id }
        Source(place.nodeId, place.anchor, place.att.id, headlineOf(place.att), path, path ?: detailOf(place.att), record = true, saveAs = place.att.name)
    }
    FileScreen(source, nav, modifier)
}

private val NOTHING_MORE: suspend (Long, Long) -> AttachmentSlice = { _, _ -> AttachmentSlice.Failed("Nothing more to read.") }

// A record is a transcript's attachment, held to its 8 MiB and fetched whole; a path is read a
// slice at a time, and only a picture is ever fetched to its end before it is shown.
private suspend fun first(io: PaneIo, held: HeldBytes?, source: Source): AttachmentSlice {
    if (!source.record) return io.range(source.anchor, source.id, 0, SLICE_BYTES - 1)
    return when (val got = held?.get(source.anchor, source.id) ?: io.attachment(source.anchor, source.id)) {
        is AttachmentBytes.Ok -> AttachmentSlice.Ok(got.bytes, got.mime, got.bytes.size.toLong())
        is AttachmentBytes.Failed -> AttachmentSlice.Failed(got.reason, got.missing)
    }
}

private suspend fun rest(io: PaneIo, source: Source, first: AttachmentSlice.Ok): ByteArray? {
    val total = first.total ?: return null
    if (total > Int.MAX_VALUE) return null
    val whole = runCatching { ByteArray(total.toInt()) }.getOrNull() ?: return null
    first.bytes.copyInto(whole)
    var have = first.bytes.size
    while (have < total) {
        val got = io.range(source.anchor, source.id, have.toLong(), total - 1) as? AttachmentSlice.Ok ?: return null
        if (got.bytes.isEmpty()) return null
        got.bytes.copyInto(whole, have, 0, minOf(got.bytes.size, whole.size - have))
        have += got.bytes.size
    }
    return whole
}

@Composable
private fun FileScreen(source: Source, nav: FilesNav, modifier: Modifier) {
    val tokens = Kampr.tokens
    val io = LocalPaneIo.current
    val held = LocalHeldBytes.current
    var fetch by remember(source.id) { mutableStateOf<Fetch>(Fetch.Asking) }
    LaunchedEffect(source.anchor, source.id) {
        fetch = when (val got = first(io, held, source)) {
            is AttachmentSlice.Ok -> {
                val more: suspend (Long, Long) -> AttachmentSlice =
                    if (source.record) NOTHING_MORE else { start, end -> io.range(source.anchor, source.id, start, end) }
                Fetch.Got(withContext(Dispatchers.Default) { openedOf(source.name, got, more) { rest(io, source, got) } })
            }

            is AttachmentSlice.Failed -> {
                val dir = source.path?.takeUnless { source.record }
                if (dir != null && io.attachment(source.anchor, lsAttachmentId(dir)) is AttachmentBytes.Ok) {
                    nav.replace(FilesPlace.Dir(source.nodeId, source.anchor, dir))
                    return@LaunchedEffect
                }
                Fetch.Failed(got.reason)
            }
        }
    }
    val opened = (fetch as? Fetch.Got)?.opened

    Column(modifier.fillMaxSize().background(tokens.color.bg)) {
        FilesBar(source.name, source.detail, nav) {
            if (opened is Opened.Words) CopyAll(source.name, opened.stream)
            source.path?.substringBeforeLast('/', "")?.let { parent ->
                BarText("Folder", "Show the folder holding ${source.name}", tokens.color.dim) {
                    nav.open(FilesPlace.Dir(source.nodeId, source.anchor, parent.ifEmpty { "/" }))
                }
            }
        }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            when (val f = fetch) {
                Fetch.Asking -> Note("asking the node for ${source.name}", tokens.color.working)

                is Fetch.Failed -> Note(f.reason, tokens.color.blocked)

                is Fetch.Got -> when (val o = f.opened) {
                    is Opened.Picture -> ZoomImage(o.image, source.name)
                    is Opened.Words -> TextFile(source.anchor, source.name, source.path, o.stream)
                    is Opened.Opaque -> Note(o.why, tokens.color.dim)
                }
            }
        }
        if (opened != null) DownloadBar(source, opened)
    }
}

@Composable
private fun Note(words: String, tint: Color) {
    KText(words, Kampr.tokens.type.caption, tint, Modifier.fillMaxWidth().padding(16.dp).announce(words), maxLines = 6)
}
