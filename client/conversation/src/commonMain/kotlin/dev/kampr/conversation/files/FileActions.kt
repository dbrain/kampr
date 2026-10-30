package dev.kampr.conversation.files

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.text.selection.DisableSelection
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.unit.dp
import dev.kampr.shared.platform.attachmentFileName
import dev.kampr.shared.platform.saveToDevice
import dev.kampr.shared.theme.Kampr
import dev.kampr.shared.ui.KText
import dev.kampr.shared.ui.LANDSCAPE_TOUCH
import dev.kampr.shared.ui.LocalPaneIo
import dev.kampr.shared.ui.LocalSafeArea
import dev.kampr.shared.ui.QuietAction
import dev.kampr.shared.ui.action
import dev.kampr.shared.ui.announce
import dev.kampr.shared.ui.touchable
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private sealed interface Copy {
    data object Idle : Copy
    data object Fetching : Copy
    data class Done(val said: String) : Copy
}

// Copy takes the file, not the lines on screen: what has not been read yet is read first, up to
// what a clipboard is for. Past that it takes what is here and says how much that was.
@Composable
internal fun CopyAll(name: String, stream: TextStream) {
    val tokens = Kampr.tokens

    // LocalClipboard replaces this, but its ClipEntry can only be built from a platform-native
    // object, so a plain string still has no common-code path in CMP 1.11.
    @Suppress("DEPRECATION")
    val clipboard = LocalClipboardManager.current
    val scope = rememberCoroutineScope()
    var copy by remember(stream) { mutableStateOf<Copy>(Copy.Idle) }
    val reachable = stream.version.let { stream.complete } || (stream.total ?: Long.MAX_VALUE) <= COPY_MOST
    val label = when (val c = copy) {
        is Copy.Done -> c.said

        Copy.Fetching -> "Reading the rest of $name to copy it"

        Copy.Idle -> when {
            stream.complete -> "Copy $name"
            reachable -> "Copy all of $name"
            else -> "Copy the ${stream.lines.lines.size} lines of $name read so far"
        }
    }
    val shown = when (copy) {
        is Copy.Done -> "Copied"
        Copy.Fetching -> "Copying…"
        Copy.Idle -> "Copy"
    }
    BarText(shown, label, if (copy is Copy.Done) tokens.color.done else tokens.color.dim) {
        if (copy != Copy.Idle) return@BarText
        scope.launch {
            copy = Copy.Fetching
            val all = if (reachable) stream.all() else stream.complete
            clipboard.setText(AnnotatedString(stream.lines.text()))
            copy = Copy.Done(if (all) "Copied" else "Copied the first ${stream.lines.lines.size} lines")
            delay(1400)
            copy = Copy.Idle
        }
    }
}

@Composable
internal fun BarText(text: String, label: String, tint: Color, onClick: () -> Unit) {
    Box(
        Modifier.touchable(LANDSCAPE_TOUCH).action(label, onClick).padding(horizontal = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        KText(text, Kampr.tokens.type.buttonSmall, tint)
    }
}

private sealed interface Saving {
    data object Idle : Saving
    data object Busy : Saving
    data class Saved(val where: String) : Saving
    data object Refused : Saving
}

// Bytes already in hand are written as they are; anything else streams from the node straight to
// the device's downloads, so a file larger than this device's memory can still be saved.
@Composable
internal fun DownloadBar(source: Source, opened: Opened) {
    val tokens = Kampr.tokens
    val safe = LocalSafeArea.current
    val io = LocalPaneIo.current
    val scope = rememberCoroutineScope()
    var saving by remember(source.id) { mutableStateOf<Saving>(Saving.Idle) }
    DisableSelection {
        Box(
            Modifier
                .fillMaxWidth()
                .background(tokens.color.bar)
                .padding(start = 12.dp + safe.left, end = 12.dp + safe.right, top = 8.dp, bottom = 8.dp + safe.bottom),
        ) {
            when (val s = saving) {
                is Saving.Saved -> KText(
                    "saved to ${s.where}",
                    tokens.type.micro,
                    tokens.color.done,
                    Modifier.fillMaxWidth().announce("Saved to ${s.where}"),
                    maxLines = 2,
                )

                else -> Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    if (s == Saving.Refused) {
                        KText("This device would not take the file.", tokens.type.micro, tokens.color.blocked, Modifier.announce("This device would not take the file."))
                    }
                    QuietAction(
                        if (s == Saving.Busy) "Downloading…" else "Download",
                        {
                            if (saving == Saving.Busy) return@QuietAction
                            saving = Saving.Busy
                            scope.launch {
                                val name = attachmentFileName(source.saveAs, opened.mime, source.id)
                                val bytes = opened.held
                                val where = if (bytes != null) {
                                    withContext(Dispatchers.Default) { saveToDevice(name, opened.mime, bytes) }
                                } else {
                                    io.download(source.anchor, source.id, name)
                                }
                                saving = if (where == null) Saving.Refused else Saving.Saved(where)
                            }
                        },
                        Modifier.fillMaxWidth(),
                        label = "Download ${source.name} to this device",
                    )
                }
            }
        }
    }
}
