package dev.kampr.conversation.files

import androidx.compose.runtime.staticCompositionLocalOf
import dev.kampr.shared.net.UploadResult
import dev.kampr.shared.net.UploadSource
import dev.kampr.shared.net.fileAttachmentId
import dev.kampr.shared.net.zipAttachmentId
import dev.kampr.shared.platform.filePickAvailable
import dev.kampr.shared.platform.pickUpload
import dev.kampr.shared.ui.Glyph
import dev.kampr.shared.ui.Icon
import dev.kampr.shared.ui.PaneIo

val LocalUploadPicker = staticCompositionLocalOf<(suspend () -> UploadSource?)?> {
    if (filePickAvailable) ::pickUpload else null
}

object TransferIcons {
    val upload = Icon(16f, 1.6f, listOf(Glyph.Trace("M8 10.2V2.6M4.8 5.6 8 2.2l3.2 3.4"), Glyph.Trace("M2.6 13.2h10.8")))
    val zip = Icon(
        16f,
        1.6f,
        listOf(Glyph.Trace("M3.2 1.8h9.6v12.4H3.2Z"), Glyph.Trace("M8 1.8v1.6M8 5v1.6M8 8.2v1.6"), Glyph.Trace("M6.6 11.4h2.8v2.8H6.6Z")),
    )
}

sealed interface Transfer {
    val words: String

    data class Moving(override val words: String) : Transfer

    data class Done(override val words: String) : Transfer

    data class Failed(override val words: String) : Transfer
}

fun zipName(dir: String): String = "${baseName(dir).takeUnless { it == "/" } ?: "root"}.zip"

fun uploadProgress(name: String, sent: Long, size: Long?): String = "Uploading $name: ${formatBytes(sent)}${size?.let { " of ${formatBytes(it)}" } ?: ""}"

fun uploadOutcome(result: UploadResult): Transfer = when (result) {
    is UploadResult.Ok -> Transfer.Done("Uploaded ${baseName(result.path)} (${formatBytes(result.bytes)})")
    is UploadResult.Failed -> Transfer.Failed("The upload did not land. ${result.reason}")
}

suspend fun saveFile(io: PaneIo, anchor: String, path: String): Transfer {
    val name = baseName(path)
    return io.download(anchor, fileAttachmentId(path), name)?.let { Transfer.Done("Saved $name to $it") }
        ?: Transfer.Failed("$name did not arrive: the node would not send it, or this device would not take it.")
}

suspend fun saveZip(io: PaneIo, anchor: String, dir: String): Transfer {
    val name = zipName(dir)
    return io.download(anchor, zipAttachmentId(dir), name)?.let { Transfer.Done("Saved $name to $it") }
        ?: Transfer.Failed("$name did not arrive: the node would not zip that folder, or this device would not take it.")
}
