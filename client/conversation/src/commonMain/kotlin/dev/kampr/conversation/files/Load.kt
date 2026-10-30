package dev.kampr.conversation.files

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.ui.LocalPaneIo
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

sealed interface Load<out T> {
    data object Loading : Load<Nothing>

    data class Failed(val reason: String) : Load<Nothing>

    data class Ready<T>(val value: T) : Load<T>
}

// The node gives one 404 for a missing path, an unreadable one, a path outside a work tree and a
// device whose role may not browse, so the sentence has to cover all of them. `AttachmentApi`
// words its 404 for a transcript record, which is the wrong sentence here.
const val NOT_BROWSABLE = "Not found, not readable, or this device may not browse files."

fun browseFailure(got: AttachmentBytes.Failed): String = if (got.missing) NOT_BROWSABLE else got.reason

@Composable
fun <T : Any> rememberLoad(anchor: String, id: String, attempt: Int, decode: (AttachmentBytes.Ok) -> T?): Load<T> {
    val io = LocalPaneIo.current
    var load by remember(anchor, id) { mutableStateOf<Load<T>>(Load.Loading) }
    LaunchedEffect(anchor, id, attempt) {
        if (load !is Load.Ready) load = Load.Loading
        load = when (val got = io.attachment(anchor, id)) {
            is AttachmentBytes.Failed -> Load.Failed(browseFailure(got))

            is AttachmentBytes.Ok -> withContext(Dispatchers.Default) { decode(got) }
                ?.let { Load.Ready(it) }
                ?: Load.Failed("The node answered with something this app cannot read. It may need updating.")
        }
    }
    return load
}

fun textOf(got: AttachmentBytes.Ok): String? = runCatching { got.bytes.decodeToString(throwOnInvalidSequence = true) }.getOrNull()
