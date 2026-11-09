package dev.kampr.conversation.files

import androidx.compose.ui.graphics.ImageBitmap
import dev.kampr.conversation.audioType
import dev.kampr.conversation.sizeWords
import dev.kampr.conversation.videoType
import dev.kampr.shared.net.AttachmentSlice
import dev.kampr.shared.platform.decodeImage

// `held` is the whole file when it is in hand, which is what Download writes rather than asking
// the node for it again; null when only a slice of it ever crossed.
sealed interface Opened {
    val held: ByteArray?
    val mime: String?

    class Picture(val image: ImageBitmap, override val held: ByteArray, override val mime: String?) : Opened
    class Words(val stream: TextStream, override val held: ByteArray?, override val mime: String?) : Opened
    class Archive(val listing: ZipListing.Ok, override val held: ByteArray?, override val mime: String?) : Opened
    class Opaque(val why: String, override val held: ByteArray?, override val mime: String?) : Opened

    // Nothing of either is read past the slice that said what it is: the player streams it.
    class Sound(override val held: ByteArray?, override val mime: String?) : Opened
    class Film(override val held: ByteArray?, override val mime: String?) : Opened
}

private fun opaque(name: String, size: Long?, mime: String?, held: ByteArray?) = Opened.Opaque(
    "$name is ${size?.let(::sizeWords) ?: "an unknown size"} of ${mime ?: "bytes"} this device cannot show. Download it to open it elsewhere.",
    held,
    mime,
)

private fun unlisted(name: String, size: Long, why: String, mime: String?, held: ByteArray?) = Opened.Opaque(
    "$name is a ${sizeWords(size)} zip archive whose contents could not be listed: $why. Download it to open it elsewhere.",
    held,
    mime,
)

private fun ByteArray.cut(start: Long, endInclusive: Long): ByteArray? = if (start < 0 || start >= size || endInclusive < start) null else copyOfRange(start.toInt(), minOf(endInclusive + 1, size.toLong()).toInt())

private suspend fun archiveOf(name: String, size: Long, first: AttachmentSlice.Ok, whole: ByteArray?, more: suspend (Long, Long) -> AttachmentSlice): Opened {
    val listing = listZip(size) { start, end ->
        if (whole != null || end < first.bytes.size) first.bytes.cut(start, end) else (more(start, end) as? AttachmentSlice.Ok)?.bytes
    }
    return when (listing) {
        is ZipListing.Ok -> Opened.Archive(listing, whole, first.mime)
        is ZipListing.Failed -> unlisted(name, size, listing.why, first.mime, whole)
    }
}

private fun undrawable(name: String, size: Long, mime: String?, held: ByteArray?) = Opened.Opaque(
    "$name is a ${sizeWords(size)} picture this device could not draw — it may be too large to decode here. Download it to open it elsewhere.",
    held,
    mime,
)

// What came back decides, not what the name promised: a client-minted id carries nothing but a
// path, and the route sniffs a media type from the bytes. A `.md` that is really a PNG is shown as
// one, and bytes that are neither are offered as a download rather than drawn as a blank viewer.
//
// From the first slice of a file: text streams on from it, an archive is listed from its tail, a
// recording or film this device can decode is streamed by its player, and a picture needs all of
// it, so `rest` fetches what is missing — or answers null when it could not be had or held.
suspend fun openedOf(
    name: String,
    first: AttachmentSlice.Ok,
    more: suspend (start: Long, endInclusive: Long) -> AttachmentSlice,
    playable: (String?) -> Boolean,
    rest: suspend () -> ByteArray?,
): Opened {
    val size = first.total ?: first.bytes.size.toLong()
    val whole = first.bytes.takeIf { it.size.toLong() >= size }
    val mime = first.mime
    if (audioType(mime) != null && playable(mime)) return Opened.Sound(whole, mime)
    if (videoType(mime) != null && playable(mime)) return Opened.Film(whole, mime)
    if (mime?.startsWith("image/") == true) {
        val bytes = whole ?: rest() ?: return undrawable(name, size, mime, null)
        return decodeImage(bytes)?.let { Opened.Picture(it, bytes, mime) } ?: undrawable(name, size, mime, bytes)
    }
    if (looksLikeZip(first.bytes)) return archiveOf(name, size, first, whole, more)
    textEncoding(first.bytes)?.let { return Opened.Words(TextStream(first, it, more), whole, mime) }
    if (whole != null) decodeImage(whole)?.let { return Opened.Picture(it, whole, mime) }
    return opaque(name, size, mime, whole)
}
