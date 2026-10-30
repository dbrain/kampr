package dev.kampr.conversation.files

import androidx.compose.ui.graphics.ImageBitmap
import dev.kampr.conversation.sizeWords
import dev.kampr.shared.net.AttachmentSlice
import dev.kampr.shared.platform.decodeImage

// `held` is the whole file when it is in hand, which is what Download writes rather than asking
// the node for it again; null when only a slice of it ever crossed.
sealed interface Opened {
    val held: ByteArray?
    val mime: String?

    class Picture(val image: ImageBitmap, override val held: ByteArray, override val mime: String?) : Opened
    class Words(val stream: TextStream, override val held: ByteArray?, override val mime: String?) : Opened
    class Opaque(val why: String, override val held: ByteArray?, override val mime: String?) : Opened
}

private fun opaque(name: String, size: Long?, mime: String?, held: ByteArray?) = Opened.Opaque(
    "$name is ${size?.let(::sizeWords) ?: "an unknown size"} of ${mime ?: "bytes"} this device cannot show. Download it to open it elsewhere.",
    held,
    mime,
)

private fun undrawable(name: String, size: Long, mime: String?, held: ByteArray?) = Opened.Opaque(
    "$name is a ${sizeWords(size)} picture this device could not draw — it may be too large to decode here. Download it to open it elsewhere.",
    held,
    mime,
)

// What came back decides, not what the name promised: a client-minted id carries nothing but a
// path, and the route sniffs a media type from the bytes. A `.md` that is really a PNG is shown as
// one, and bytes that are neither are offered as a download rather than drawn as a blank viewer.
//
// From the first slice of a file: text streams on from it, and a picture needs all of it, so
// `rest` fetches what is missing — or answers null when it could not be had or held.
suspend fun openedOf(
    name: String,
    first: AttachmentSlice.Ok,
    more: suspend (start: Long, endInclusive: Long) -> AttachmentSlice,
    rest: suspend () -> ByteArray?,
): Opened {
    val size = first.total ?: first.bytes.size.toLong()
    val whole = first.bytes.takeIf { it.size.toLong() >= size }
    val mime = first.mime
    if (mime?.startsWith("image/") == true) {
        val bytes = whole ?: rest() ?: return undrawable(name, size, mime, null)
        return decodeImage(bytes)?.let { Opened.Picture(it, bytes, mime) } ?: undrawable(name, size, mime, bytes)
    }
    if (looksLikeText(first.bytes)) return Opened.Words(TextStream(first, more), whole, mime)
    if (whole != null) decodeImage(whole)?.let { return Opened.Picture(it, whole, mime) }
    return opaque(name, size, mime, whole)
}
