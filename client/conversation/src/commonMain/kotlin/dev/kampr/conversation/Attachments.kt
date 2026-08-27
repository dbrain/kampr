package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.ImageBitmap
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.platform.attachmentFileName
import dev.kampr.shared.platform.decodeImage
import dev.kampr.shared.platform.saveToDevice
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlin.math.round

private const val IMAGE = "image"
private const val VIDEO = "video"

// A kind this release has never heard of is a file, offered as a download. Matching an exhaustive
// set instead is what would make the next kind the node learns to produce vanish out of a
// transcript on every phone that is already installed.
enum class AttachmentOffer(val label: String) {
    Image("Show image"),
    Video("Show video"),
    File("Download file"),
}

fun offerFor(att: Attachment): AttachmentOffer = when (att.kind) {
    IMAGE -> AttachmentOffer.Image
    VIDEO -> AttachmentOffer.Video
    else -> AttachmentOffer.File
}

sealed interface AttachmentState {
    data object Idle : AttachmentState
    data object Fetching : AttachmentState
    // The bytes are kept beside the picture, and they are the cheap half: a 730 KB screenshot is
    // ~9 MB once decoded, so holding the original as well costs a twelfth of what is already held
    // and is the only way the viewer can hand the file to the device afterwards. Re-fetching it
    // would be a second authorised round trip for bytes that never left.
    data class Shown(val image: ImageBitmap, val bytes: ByteArray, val mime: String?) : AttachmentState {
        override fun equals(other: Any?): Boolean = this === other
        override fun hashCode(): Int = image.hashCode()
    }
    data class Saved(val where: String) : AttachmentState
    data class Failed(val reason: String) : AttachmentState
}

// A pasted screenshot has no filename, so the headline is what it *is* and the name is a bonus.
fun headlineOf(att: Attachment): String = att.name?.takeIf { it.isNotBlank() }
    ?: when (offerFor(att)) {
        AttachmentOffer.Image -> "Image"
        AttachmentOffer.Video -> "Video"
        AttachmentOffer.File -> "File"
    }

fun detailOf(att: Attachment): String? {
    val type = att.mime?.takeIf { it.isNotBlank() }?.let { mime ->
        if (mime.startsWith("$IMAGE/") || mime.startsWith("$VIDEO/")) mime.substringAfter('/') else mime
    }
    return listOfNotNull(type, att.bytes?.takeIf { it > 0 }?.let(::sizeWords)).joinToString(" · ")
        .takeIf { it.isNotEmpty() }
}

fun sizeWords(bytes: Long): String = when {
    bytes < 1_000 -> "$bytes B"
    bytes < 1_000_000 -> "${oneDecimal(bytes / 1_000.0)} KB"
    else -> "${oneDecimal(bytes / 1_000_000.0)} MB"
}

private fun oneDecimal(value: Double): String {
    val tenths = round(value * 10).toLong()
    return if (tenths % 10 == 0L) "${tenths / 10}" else "${tenths / 10}.${tenths % 10}"
}

// A 730 KB screenshot is ~9 MB of pixels once it is decoded, and a long transcript mentions many.
// Holding the four most recently opened, and no more than 24 MB of them, keeps a reader who is
// paging back through a day's images from walking the phone into an out-of-memory kill; an image
// that falls out is not lost, it is a button again and one press from coming back.
private const val MOST_IMAGES_HELD = 4
private const val MOST_PIXEL_BYTES_HELD = 24L * 1024 * 1024

@Stable
class AttachmentStore(
    private val pane: String,
    private val mostImagesHeld: Int = MOST_IMAGES_HELD,
    private val mostPixelBytesHeld: Long = MOST_PIXEL_BYTES_HELD,
) {
    private val states = mutableStateMapOf<String, AttachmentState>()
    private val savedTo = mutableStateMapOf<String, String>()

    // Which picture is open over the transcript, if any. Held here rather than in the card that
    // opened it: the viewer covers the whole pane, and a card inside a lazy list is composed away
    // the moment the thing it opened scrolls off.
    var viewing: Attachment? by mutableStateOf(null)
        private set
    private val held = ArrayDeque<String>()
    private val pixelBytes = HashMap<String, Long>()

    fun state(id: String): AttachmentState = states[id] ?: AttachmentState.Idle

    fun saved(id: String): String? = savedTo[id]

    fun view(att: Attachment) {
        viewing = att
    }

    fun close() {
        viewing = null
    }

    // Only what is already in hand. A picture that has fallen out of the held set is a button
    // again, and the button fetches before it can be looked at, so there is no path here that
    // saves bytes this store does not have.
    //
    // Off the main thread, like the fetch that put the bytes there: `saveToDevice` opens a
    // MediaStore row and writes the whole file, and doing that under a click handler is a frame
    // budget spent on a syscall.
    suspend fun save(att: Attachment) {
        val shown = states[att.id] as? AttachmentState.Shown ?: return
        val name = attachmentFileName(att.name, shown.mime, att.id)
        val where = withContext(Dispatchers.Default) { saveToDevice(name, shown.mime, shown.bytes) } ?: return
        savedTo[att.id] = where
    }

    suspend fun open(io: PaneIo, att: Attachment) {
        if (state(att.id) == AttachmentState.Fetching) return
        states[att.id] = AttachmentState.Fetching
        val landed = when (val got = io.attachment(pane, att.id)) {
            is AttachmentBytes.Failed -> AttachmentState.Failed(got.reason)
            is AttachmentBytes.Ok -> withContext(Dispatchers.Default) { received(att, got) }
        }
        if (landed is AttachmentState.Shown) hold(att.id, landed) else states[att.id] = landed
    }

    private fun received(att: Attachment, got: AttachmentBytes.Ok): AttachmentState {
        if (offerFor(att) != AttachmentOffer.Image) {
            val mime = att.mime ?: got.mime
            val where = saveToDevice(attachmentFileName(att.name, mime, att.id), mime, got.bytes)
                ?: return AttachmentState.Failed("This device would not take the file.")
            return AttachmentState.Saved(where)
        }
        val image = decodeImage(got.bytes)
            ?: return AttachmentState.Failed("Those bytes are not a picture this device can read.")
        return AttachmentState.Shown(image, got.bytes, att.mime ?: got.mime)
    }

    private fun hold(id: String, shown: AttachmentState.Shown) {
        states[id] = shown
        held.remove(id)
        held.addLast(id)
        pixelBytes[id] = shown.image.width.toLong() * shown.image.height.toLong() * 4
        while (held.size > 1 && (held.size > mostImagesHeld || pixelBytes.values.sum() > mostPixelBytesHeld)) {
            val oldest = held.removeFirst()
            pixelBytes.remove(oldest)
            states.remove(oldest)
        }
    }
}

@Composable
fun rememberAttachmentStore(pane: String): AttachmentStore = remember(pane) { AttachmentStore(pane) }
