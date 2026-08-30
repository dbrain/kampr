package dev.kampr.shared.platform

import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.net.URLConnection
import javax.swing.JFileChooser

// A desktop always has a chooser. Whether *this* JVM can raise one is a question about the
// window server rather than about the platform, and a picker that cannot open answers the same
// thing a picker somebody backed out of answers: nothing was chosen.
actual val filePickAvailable: Boolean = true

actual suspend fun pickFile(): PickedFile? = withContext(Dispatchers.IO) {
    runCatching {
        val chooser = JFileChooser()
        if (chooser.showOpenDialog(null) != JFileChooser.APPROVE_OPTION) return@runCatching null
        val file: File = chooser.selectedFile ?: return@runCatching null
        val bytes = file.readBytes().takeIf { it.isNotEmpty() } ?: return@runCatching null
        PickedFile(file.name, URLConnection.guessContentTypeFromName(file.name), bytes)
    }.getOrNull()
}

// AWT has no paste event: a desktop's clipboard can only be read when something asks, and nothing
// here is told that a paste happened. The chooser above is the whole of the desktop's answer.
actual suspend fun pastedFile(): PickedFile? = null

@Composable
actual fun Modifier.acceptsPastedFiles(): Modifier = this
