package dev.kampr.shared.platform

import android.content.Context
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
