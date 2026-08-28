package dev.kampr.terminal.file

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.graphics.ImageBitmap
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.platform.attachmentFileName
import dev.kampr.shared.platform.decodeImage
import dev.kampr.shared.platform.saveToDevice
import dev.kampr.shared.ui.PaneIo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

private const val IMAGE = "image/"

sealed interface Peeked {
    data object Fetching : Peeked
    data class Words(val text: String) : Peeked
    data class Picture(val image: ImageBitmap) : Peeked
    data class Saved(val where: String) : Peeked
    data class Failed(val reason: String) : Peeked
}

fun fileName(path: String): String = path.trimEnd('/').substringAfterLast('/').ifEmpty { path }

// One path at a time, and nothing held once it is closed: a grid pane already holds a scrollback
// and a run-layout cache, and an 8 MiB file kept behind a closed sheet is that budget spent twice.
@Stable
class FilePeek {
    var path by mutableStateOf<String?>(null)
        private set

    var state by mutableStateOf<Peeked>(Peeked.Fetching)
        private set

    suspend fun open(io: PaneIo, paneId: String, at: String) {
        path = at
        state = Peeked.Fetching
        val landed = when (val got = io.attachment(paneId, fileAttachmentId(at))) {
            is AttachmentBytes.Failed -> Peeked.Failed(got.reason)
            is AttachmentBytes.Ok -> withContext(Dispatchers.Default) { read(at, got) }
        }
        if (path == at) state = landed
    }

    fun close() {
        path = null
        state = Peeked.Fetching
    }
}

// What came back decides, not what the extension promised: a client-minted id carries nothing but
// a path, and the route answers a media type sniffed from the bytes. So a `.md` that is really a
// PNG is shown as one, and bytes that are neither go to the device rather than to a blank viewer.
private fun read(path: String, got: AttachmentBytes.Ok): Peeked {
    if (got.mime?.startsWith(IMAGE) == true) decodeImage(got.bytes)?.let { return Peeked.Picture(it) }
    runCatching { got.bytes.decodeToString(throwOnInvalidSequence = true) }.getOrNull()
        ?.let { return Peeked.Words(it) }
    decodeImage(got.bytes)?.let { return Peeked.Picture(it) }
    val where = saveToDevice(attachmentFileName(fileName(path), got.mime, path), got.mime, got.bytes)
    return if (where == null) Peeked.Failed("This device would not take the file.") else Peeked.Saved(where)
}
