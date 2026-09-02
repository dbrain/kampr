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

// Kept for `detailOf`, which shortens `video/mp4` to `mp4` in a label. There is no video *offer* —
// see `offerFor`. Naming a type is not the same as promising to open it.
private const val VIDEO = "video"
private const val AUDIO = "audio"
private const val TEXT = "text"

// A kind this release has never heard of is a file, offered as a download. Matching an exhaustive
// set instead is what would make the next kind the node learns to produce vanish out of a
// transcript on every phone that is already installed.
enum class AttachmentOffer(val label: String) {
    Image("Show image"),
    Audio("Play audio"),
    Text("Show file"),
    File("Download file"),
}

// **There is deliberately no video offer.** One existed, reachable only from `kind == "video"`,
// which nothing in the tree has ever written — and had a node begun to, the reader would have been
// shown "Show video" over a route that serves no video type inline and a client with no player, so
// it was a present-and-failing affordance waiting for its producer. Video falls through to the
// download every unknown kind gets, which is the truth until something can actually play one.
fun offerFor(att: Attachment): AttachmentOffer = when {
    att.kind == IMAGE -> AttachmentOffer.Image
    att.kind == AUDIO || att.mime?.startsWith("$AUDIO/") == true -> AttachmentOffer.Audio
    // A file whose bytes are words is read here rather than handed to the device's downloads
    // folder, which on a phone is a file the reader then has to go and find an app for.
    att.kind == TEXT || att.mime?.startsWith("$TEXT/") == true -> AttachmentOffer.Text
    else -> AttachmentOffer.File
}

// What *this* device will do with it. [offerFor] says what the attachment is, and that is the
// half a test can pin down; whether there is a decoder behind the word "play" is the other half,
// and a reader told to press play who gets a file in their downloads folder was lied to. Only
// audio can lose its offer this way — every other kind reaches the same download in the end.
//
// `named` is the path the target was built from, for a client-minted id that carries no `mime`.
fun offerOn(att: Attachment, voices: Voices, named: String? = null): AttachmentOffer {
    val offer = offerFor(att)
    if (offer != AttachmentOffer.Audio) return offer
    val type = audioType(att.mime) ?: soundType(named ?: att.name)
    return if (voices.canPlay(type)) offer else AttachmentOffer.File
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
    // Held beside the text for the same reason a picture's are: the viewer hands the file to the
    // device afterwards, and re-fetching bytes that never left is a second authorised round trip
    // for nothing.
    data class Text(val text: String, val bytes: ByteArray, val mime: String?) : AttachmentState {
        override fun equals(other: Any?): Boolean = this === other
        override fun hashCode(): Int = text.hashCode()
    }
    // The bytes and nothing made of them. A [Voice] is a decoder and, on two of the three targets,
    // a hardware line: it is opened by the card that draws the button and released when that card
    // leaves the composition, so nothing this store evicts can leave one running.
    data class Sound(val bytes: ByteArray, val mime: String?) : AttachmentState {
        override fun equals(other: Any?): Boolean = this === other
        override fun hashCode(): Int = bytes.size
    }
    data class Saved(val where: String) : AttachmentState
    data class Failed(val reason: String) : AttachmentState
}

// A pasted screenshot has no filename, so the headline is what it *is* and the name is a bonus.
fun headlineOf(att: Attachment): String = att.name?.takeIf { it.isNotBlank() }
    ?: when (offerFor(att)) {
        AttachmentOffer.Image -> "Image"
        AttachmentOffer.Audio -> "Audio"
        AttachmentOffer.Text -> "File"
        AttachmentOffer.File -> "File"
    }

fun detailOf(att: Attachment): String? {
    val type = att.mime?.takeIf { it.isNotBlank() }?.let { mime ->
        if (mime.startsWith("$IMAGE/") || mime.startsWith("$VIDEO/") || mime.startsWith("$AUDIO/")) {
            mime.substringAfter('/')
        } else {
            mime
        }
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
//
// A fetched file is held against the same budget for the same reason: the route will hand back
// 8 MiB of one, and a reader opening every path an agent touched would otherwise hold all of them.
private const val MOST_IMAGES_HELD = 4
private const val MOST_PIXEL_BYTES_HELD = 24L * 1024 * 1024

@Stable
class AttachmentStore(
    private val pane: String,
    private val voices: Voices = deviceVoices,
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
        val (bytes, mime) = when (val held = states[att.id]) {
            is AttachmentState.Shown -> held.bytes to held.mime
            is AttachmentState.Text -> held.bytes to held.mime
            is AttachmentState.Sound -> held.bytes to held.mime
            else -> return
        }
        val name = attachmentFileName(att.name, mime, att.id)
        val where = withContext(Dispatchers.Default) { saveToDevice(name, mime, bytes) } ?: return
        savedTo[att.id] = where
    }

    // One press for a target the reader named themselves: they asked for the file, so the fetch
    // and the opening of it are one action rather than two.
    suspend fun reveal(io: PaneIo, att: Attachment) {
        open(io, att)
        if (state(att.id).let { it is AttachmentState.Shown || it is AttachmentState.Text }) view(att)
    }

    suspend fun open(io: PaneIo, att: Attachment) {
        if (state(att.id) == AttachmentState.Fetching) return
        states[att.id] = AttachmentState.Fetching
        val landed = when (val got = io.attachment(pane, att.id)) {
            is AttachmentBytes.Failed -> AttachmentState.Failed(got.reason)
            is AttachmentBytes.Ok -> withContext(Dispatchers.Default) { received(att, got) }
        }
        when (landed) {
            is AttachmentState.Shown -> hold(att.id, landed, landed.image.width.toLong() * landed.image.height * 4)
            is AttachmentState.Text -> hold(att.id, landed, landed.bytes.size.toLong())
            is AttachmentState.Sound -> hold(att.id, landed, landed.bytes.size.toLong())
            else -> states[att.id] = landed
        }
    }

    // What came back decides, not what the header promised: the route answers a media type from
    // the bytes when the record named none, and a path this client built an id for has nothing but
    // an extension behind its guess. So an image that will not decode and a file that is not text
    // both fall through to the download rather than to a blank viewer.
    private fun received(att: Attachment, got: AttachmentBytes.Ok): AttachmentState {
        val mime = att.mime ?: got.mime
        val offer = offerFor(att)
        if (offer == AttachmentOffer.Image || mime?.startsWith("$IMAGE/") == true) {
            decodeImage(got.bytes)?.let { return AttachmentState.Shown(it, got.bytes, mime) }
            if (offer == AttachmentOffer.Image && att.mime != null) {
                return AttachmentState.Failed("Those bytes are not a picture this device can read.")
            }
        }
        // Asked again with the type the node derived from the bytes, rather than trusted from the
        // guess an extension made before the fetch: a `.wav` that is really an MP3 is a device
        // that cannot play it, and the honest end of that is the download below.
        if (offer == AttachmentOffer.Audio && voices.canPlay(mime)) {
            return AttachmentState.Sound(got.bytes, mime)
        }
        if (offer == AttachmentOffer.Text) {
            runCatching { got.bytes.decodeToString(throwOnInvalidSequence = true) }.getOrNull()
                ?.let { return AttachmentState.Text(it, got.bytes, mime) }
        }
        val where = saveToDevice(attachmentFileName(att.name, mime, att.id), mime, got.bytes)
            ?: return AttachmentState.Failed("This device would not take the file.")
        return AttachmentState.Saved(where)
    }

    private fun hold(id: String, landed: AttachmentState, bytes: Long) {
        states[id] = landed
        held.remove(id)
        held.addLast(id)
        pixelBytes[id] = bytes
        while (held.size > 1 && (held.size > mostImagesHeld || pixelBytes.values.sum() > mostPixelBytesHeld)) {
            val oldest = held.removeFirst()
            pixelBytes.remove(oldest)
            states.remove(oldest)
        }
    }
}

@Composable
fun rememberAttachmentStore(pane: String): AttachmentStore {
    val voices = LocalVoices.current
    return remember(pane, voices) { AttachmentStore(pane, voices) }
}
