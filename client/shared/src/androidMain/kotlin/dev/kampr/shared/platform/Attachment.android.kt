package dev.kampr.shared.platform

import android.content.ContentValues
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import java.io.File

actual fun saveToDevice(name: String, mime: String?, bytes: ByteArray): String? {
    val context = KamprAndroid.context ?: return null
    val type = mime ?: "application/octet-stream"
    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
        val values = ContentValues().apply {
            put(MediaStore.Downloads.DISPLAY_NAME, name)
            put(MediaStore.Downloads.MIME_TYPE, type)
        }
        val resolver = context.contentResolver
        val target = runCatching {
            resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values)
        }.getOrNull() ?: return null
        return runCatching {
            resolver.openOutputStream(target)?.use { it.write(bytes) } ?: return null
            "Downloads/$name"
        }.getOrNull()
    }
    // Below Q there is no Downloads collection to insert into without WRITE_EXTERNAL_STORAGE, and
    // raising a storage permission prompt to save one screenshot is a worse trade than a path
    // inside this app's own external directory, which a file manager can still reach.
    val dir = context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: return null
    return runCatching {
        val file = File(dir, name)
        file.writeBytes(bytes)
        file.absolutePath
    }.getOrNull()
}
