package dev.kampr.conversation

import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.platform.ClipEntry
import androidx.compose.ui.platform.Clipboard
import java.awt.datatransfer.DataFlavor
import java.awt.datatransfer.Transferable

// What was last copied, read back as the plain text it was put there as.
internal class HeldClipboard : Clipboard {
    private var entry: ClipEntry? = null

    override suspend fun getClipEntry(): ClipEntry? = entry

    override suspend fun setClipEntry(clipEntry: ClipEntry?) {
        entry = clipEntry
    }

    @OptIn(ExperimentalComposeUiApi::class)
    val held: String?
        get() = (entry?.nativeClipEntry as? Transferable)?.getTransferData(DataFlavor.stringFlavor) as String?
}
