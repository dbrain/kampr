package dev.kampr.shared.platform

import androidx.compose.ui.graphics.ImageBitmap
import org.jetbrains.compose.resources.decodeToImageBitmap

// Null means "these bytes are not a picture this device can read", and the caller says so on
// screen. Compose Multiplatform's own decoder already is the platform split — BitmapFactory on
// Android, skia everywhere else — so this is one function rather than three actuals of it.
fun decodeImage(bytes: ByteArray): ImageBitmap? = runCatching { bytes.decodeToImageBitmap() }.getOrNull()

// Where the file landed, in words an operator can go and look at, or null if this device refused
// it. A silent "saved" that saved nothing is the failure mode this return type exists to prevent.
expect fun saveToDevice(name: String, mime: String?, bytes: ByteArray): String?

// A pasted screenshot genuinely has no filename, and every platform below needs one to write.
fun attachmentFileName(name: String?, mime: String?, id: String): String {
    name?.trim()?.takeIf { it.isNotEmpty() }?.let { return it.replace('/', '-').replace('\\', '-') }
    val suffix = mime?.substringAfter('/', "")?.substringBefore('+')?.takeIf { it.isNotEmpty() }
    return if (suffix == null) "kampr-$id" else "kampr-$id.$suffix"
}
