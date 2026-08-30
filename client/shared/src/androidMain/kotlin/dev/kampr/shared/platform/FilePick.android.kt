package dev.kampr.shared.platform

import android.content.Context
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.content.consume
import androidx.compose.foundation.content.contentReceiver
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.launch
import android.net.Uri
import android.provider.OpenableColumns
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import dev.kampr.shared.net.KamprHost
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import kotlin.coroutines.resume

actual val filePickAvailable: Boolean = true

// Registered against the Activity's own result registry rather than through
// `rememberLauncherForActivityResult`, because the composer that raises this is a button in a lazy
// list and a launcher that has to survive the picker taking the window cannot be one the list may
// compose away while it is up.
private var pickers = 0

actual suspend fun pickFile(): PickedFile? {
    val activity = KamprHost.activity as? ComponentActivity ?: return null
    val uri = suspendCancellableCoroutine<Uri?> { waiting ->
        var launcher: ActivityResultLauncher<String>? = null
        launcher = activity.activityResultRegistry.register(
            "kampr-pick-${pickers++}",
            ActivityResultContracts.GetContent(),
        ) { picked ->
            launcher?.unregister()
            waiting.resume(picked)
        }
        waiting.invokeOnCancellation { launcher?.unregister() }
        launcher?.launch("*/*")
    } ?: return null
    return withContext(Dispatchers.IO) { read(activity, uri) }
}

private fun read(context: Context, uri: Uri): PickedFile? {
    val resolver = context.contentResolver
    val name = runCatching {
        resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME), null, null, null)?.use { row ->
            if (row.moveToFirst() && !row.isNull(0)) row.getString(0) else null
        }
    }.getOrNull()
    val bytes = runCatching { resolver.openInputStream(uri)?.use { it.readBytes() } }.getOrNull()
    return bytes?.takeIf { it.isNotEmpty() }?.let { PickedFile(name, resolver.getType(uri), it) }
}

// **Android delivers a pasted file to one place: the text field it was pasted into.** There is no
// page-wide event to listen for, so the queue here is fed by [`acceptsPastedFiles`] on the composer
// rather than by anything this function can install, and it simply waits — a pane with no field on
// it is a pane where no paste will ever arrive, which is a wait rather than a refusal.
//
// Buffered rather than rendezvous: the reader spends most of a paste's life encoding the previous
// file and sending it, and a clipboard that arrives during that must be kept.
private val pasted = Channel<PickedFile>(capacity = 8)

actual suspend fun pastedFile(): PickedFile? = pasted.receive()

// **Only a `BasicTextField` built on a `TextFieldState` can feed this** (#368). It is applied to
// the composer's own column rather than to the field, which is what makes the same modifier a
// drag-and-drop target as well — `ReceiveContentNode` is one in its own right, so a file dragged
// onto the reply box lands here without a text field being involved at all.
//
// **The item is claimed before its bytes are read, and it has to be.** `consume` wants an answer
// while the paste is still happening and reading a `content://` URI is IO; worse, the URI is a
// permission grant that dies with the paste, so a read deferred until after this returns reads
// nothing. So the read starts here and the item is claimed on the strength of having a URI at all
// — which is honest, because a claim this listener declines is one that falls through to the text
// field and inserts a `content://` line into somebody's reply.
@OptIn(ExperimentalFoundationApi::class)
@Composable
actual fun Modifier.acceptsPastedFiles(): Modifier {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    return contentReceiver { content ->
        content.consume { item ->
            val uri = item.uri ?: return@consume false
            scope.launch(Dispatchers.IO) { read(context, uri)?.let { pasted.send(it) } }
            true
        }
    }
}
