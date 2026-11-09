package dev.kampr.conversation

import androidx.compose.runtime.Composable
import androidx.compose.runtime.Stable
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.remember
import androidx.compose.ui.graphics.ImageBitmap
import dev.kampr.conversation.files.HeldBytes
import dev.kampr.conversation.files.LocalHeldBytes
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.platform.attachmentFileName
import dev.kampr.shared.platform.decodeImage
import dev.kampr.shared.platform.saveToDevice
import dev.kampr.shared.ui.PaneIo
import dev.kampr.shared.wire.Attachment
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.sync.Semaphore
import kotlinx.coroutines.sync.withPermit
import kotlinx.coroutines.withContext
import kotlin.math.round

private const val IMAGE = "image"

private const val VIDEO = "video"
private const val AUDIO = "audio"
private const val TEXT = "text"

// A kind this release has never heard of is a file, offered as a download. Matching an exhaustive
// set instead is what would make the next kind the node learns to produce vanish out of a
// transcript on every phone that is already installed.
enum class AttachmentOffer(val label: String) {
    Image("Show image"),
    Audio("Play audio"),
    Video("Show video"),
    Text("Show file"),
    File("Download file"),
}

fun offerFor(att: Attachment): AttachmentOffer = when {
    att.kind == IMAGE -> AttachmentOffer.Image

    att.kind == AUDIO || att.mime?.startsWith("$AUDIO/") == true -> AttachmentOffer.Audio

    att.kind == VIDEO || att.mime?.startsWith("$VIDEO/") == true -> AttachmentOffer.Video

    // A file whose bytes are words is read here rather than handed to the device's downloads
    // folder, which on a phone is a file the reader then has to go and find an app for.
    att.kind == TEXT || att.mime?.startsWith("$TEXT/") == true -> AttachmentOffer.Text

    else -> AttachmentOffer.File
}

// What *this* device will do with it. [offerFor] says what the attachment is, and that is the
// half a test can pin down; whether there is a decoder behind the word "play" is the other half,
// and a reader told to press play who gets a file in their downloads folder was lied to. Only
// audio and video can lose their offer this way — every other kind reaches the same download in
// the end.
//
// `named` is the path the target was built from, for a client-minted id that carries no `mime`.
fun offerOn(att: Attachment, voices: Voices, films: Films, named: String? = null): AttachmentOffer = when (val offer = offerFor(att)) {
    AttachmentOffer.Audio -> offer.takeIf { voices.canPlay(audioType(att.mime) ?: soundType(named ?: att.name)) }
    AttachmentOffer.Video -> offer.takeIf { films.canPlay(att.mime) }
    else -> offer
} ?: AttachmentOffer.File

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

    // Where the recording can be streamed from, and nothing made of it. A [Voice] is a decoder
    // and, on two of the three targets, a hardware line: it is opened by the card that draws the
    // button and released when that card leaves the composition, so nothing this store holds can
    // leave one running.
    data class Sound(val url: String, val mime: String?) : AttachmentState
    data class Saved(val where: String) : AttachmentState
    data class Failed(val reason: String) : AttachmentState
}

// A pasted screenshot has no filename, so the headline is what it *is* and the name is a bonus.
fun headlineOf(att: Attachment): String = att.name?.takeIf { it.isNotBlank() }
    ?: when (offerFor(att)) {
        AttachmentOffer.Image -> "Image"
        AttachmentOffer.Audio -> "Audio"
        AttachmentOffer.Video -> "Video"
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

// Tiles are small enough to keep a long day's worth, and fetched two at a time because each one is
// a full-size decode on the way to being small.
private const val MOST_THUMBNAILS_HELD = 48
private const val THUMBNAILS_AT_ONCE = 2

@Stable
class AttachmentStore(
    val pane: String,
    private val voices: Voices = deviceVoices,
    private val films: Films = deviceFilms,
    private val heldBytes: HeldBytes? = null,
    private val mostImagesHeld: Int = MOST_IMAGES_HELD,
    private val mostPixelBytesHeld: Long = MOST_PIXEL_BYTES_HELD,
) {
    private val states = mutableStateMapOf<String, AttachmentState>()
    private val savedTo = mutableStateMapOf<String, String>()

    private val held = ArrayDeque<String>()
    private val pixelBytes = HashMap<String, Long>()
    private val thumbnails = mutableStateMapOf<String, Thumbnail>()
    private val thumbnailsHeld = ArrayDeque<String>()
    private val thumbnailFetches = Semaphore(THUMBNAILS_AT_ONCE)

    fun state(id: String): AttachmentState = states[id] ?: AttachmentState.Idle

    fun saved(id: String): String? = savedTo[id]

    fun thumbnail(id: String): Thumbnail = thumbnails[id] ?: Thumbnail.Idle

    suspend fun fetchThumbnail(io: PaneIo, att: Attachment) {
        if (thumbnail(att.id).let { it is Thumbnail.Fetching || it is Thumbnail.Shown }) return
        thumbnails[att.id] = Thumbnail.Fetching
        // A tile that left the screen mid-fetch leaves its thumbnail to be asked for again rather
        // than marked as fetching for ever.
        val landed = try {
            fetchedThumbnail(io, att)
        } catch (e: CancellationException) {
            thumbnails.remove(att.id)
            throw e
        }
        thumbnails[att.id] = landed
        if (landed !is Thumbnail.Shown) return
        thumbnailsHeld.remove(att.id)
        thumbnailsHeld.addLast(att.id)
        while (thumbnailsHeld.size > MOST_THUMBNAILS_HELD) thumbnails.remove(thumbnailsHeld.removeFirst())
    }

    private suspend fun fetchedThumbnail(io: PaneIo, att: Attachment): Thumbnail = thumbnailFetches.withPermit {
        when (val got = io.attachment(pane, att.id)) {
            is AttachmentBytes.Failed -> Thumbnail.Failed(got.reason)

            is AttachmentBytes.Ok -> withContext(Dispatchers.Default) {
                heldBytes?.put(pane, att.id, got)
                decodeImage(got.bytes)?.let { Thumbnail.Shown(it.within(THUMBNAIL_PX)) }
                    ?: Thumbnail.Failed("Those bytes are not a picture this device can read.")
            }
        }
    }

    // Only what is already in hand. A picture that has fallen out of the held set is a button
    // again, and the button fetches before it can be looked at, so there is no path here that
    // saves bytes this store does not have.
    //
    // Off the main thread, like the fetch that put the bytes there: `saveToDevice` opens a
    // MediaStore row and writes the whole file, and doing that under a click handler is a frame
    // budget spent on a syscall.
    //
    // A recording was never held, so it streams from the node to the downloads instead.
    suspend fun save(io: PaneIo, att: Attachment) {
        val where = when (val held = states[att.id]) {
            is AttachmentState.Shown -> withContext(Dispatchers.Default) {
                saveToDevice(attachmentFileName(att.name, held.mime, att.id), held.mime, held.bytes)
            }

            is AttachmentState.Sound -> io.download(pane, att.id, attachmentFileName(att.name, held.mime, att.id))

            else -> return
        } ?: return
        savedTo[att.id] = where
    }

    // A ticket the node no longer holds — it restarted, or twelve hours went by — is replaced by
    // a new one, and the bar playing it stays where it is.
    suspend fun restream(io: PaneIo, att: Attachment) {
        states[att.id] = io.streamUrl(pane, att.id)
            ?.let { AttachmentState.Sound(it, audioType(att.mime) ?: soundType(att.name)) }
            ?: AttachmentState.Failed("The node could not stream ${headlineOf(att)}.")
    }

    suspend fun open(io: PaneIo, att: Attachment) {
        if (state(att.id) == AttachmentState.Fetching) return
        states[att.id] = AttachmentState.Fetching
        // A recording plays from the node rather than being fetched whole: an hour of audio is
        // hundreds of megabytes, and the player only ever needs the part it is at. One this device
        // cannot decode is the download below, like any other file.
        if (offerOn(att, voices, films) == AttachmentOffer.Audio) {
            restream(io, att)
            return
        }
        val landed = when (val got = io.attachment(pane, att.id)) {
            is AttachmentBytes.Failed -> AttachmentState.Failed(got.reason)

            is AttachmentBytes.Ok -> {
                heldBytes?.put(pane, att.id, got)
                withContext(Dispatchers.Default) { received(att, got) }
            }
        }
        when (landed) {
            is AttachmentState.Shown -> hold(att.id, landed, landed.image.width.toLong() * landed.image.height * 4)
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
    val films = LocalFilms.current
    val held = LocalHeldBytes.current
    return remember(pane, voices, films, held) { AttachmentStore(pane, voices, films, heldBytes = held) }
}
