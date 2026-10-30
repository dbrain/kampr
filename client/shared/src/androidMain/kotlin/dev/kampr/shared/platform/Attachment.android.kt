package dev.kampr.shared.platform

import android.content.ContentValues
import android.content.Context
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import dev.kampr.shared.net.DownloadSink
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File
import java.io.OutputStream

actual fun saveToDevice(name: String, mime: String?, bytes: ByteArray): String? {
    val target = Landing.open(name, mime) ?: return null
    return runCatching {
        target.out.write(bytes)
        target.finish()
    }.getOrElse {
        target.abort()
        null
    }
}

actual fun openDownload(name: String, mime: String?): DownloadSink? = Landing.open(name, mime)?.let(::StreamSink)

private class StreamSink(private val landing: Landing) : DownloadSink {
    override suspend fun write(chunk: ByteArray, length: Int) = withContext(Dispatchers.IO) { landing.out.write(chunk, 0, length) }
    override suspend fun finish(): String? = withContext(Dispatchers.IO) { runCatching { landing.finish() }.getOrNull() }
    override suspend fun abort() = withContext(Dispatchers.IO) { landing.abort() }
}

private class Landing(
    val out: OutputStream,
    private val where: String,
    private val context: Context,
    private val pending: Uri?,
    private val file: File?,
) {
    fun finish(): String {
        out.close()
        if (pending != null && Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            context.contentResolver.update(pending, ContentValues().apply { put(MediaStore.Downloads.IS_PENDING, 0) }, null, null)
        }
        return where
    }

    fun abort() {
        runCatching { out.close() }
        runCatching { pending?.let { context.contentResolver.delete(it, null, null) } }
        runCatching { file?.delete() }
    }

    companion object {
        // Pending while it is written, so a half-written download is never offered to another app
        // and one that fails is deleted rather than left as a truncated file in Downloads.
        fun open(name: String, mime: String?): Landing? {
            val context = KamprAndroid.context ?: return null
            val type = mime ?: "application/octet-stream"
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                val values = ContentValues().apply {
                    put(MediaStore.Downloads.DISPLAY_NAME, name)
                    put(MediaStore.Downloads.MIME_TYPE, type)
                    put(MediaStore.Downloads.IS_PENDING, 1)
                }
                val resolver = context.contentResolver
                val uri = runCatching { resolver.insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, values) }.getOrNull() ?: return null
                val out = runCatching { resolver.openOutputStream(uri) }.getOrNull()
                if (out == null) {
                    runCatching { resolver.delete(uri, null, null) }
                    return null
                }
                return Landing(out, "Downloads/$name", context, uri, null)
            }
            // Below Q there is no Downloads collection to insert into without WRITE_EXTERNAL_STORAGE, and
            // raising a storage permission prompt to save one screenshot is a worse trade than a path
            // inside this app's own external directory, which a file manager can still reach.
            val dir = context.getExternalFilesDir(Environment.DIRECTORY_DOWNLOADS) ?: return null
            val file = File(dir, name)
            val out = runCatching { file.outputStream() }.getOrNull() ?: return null
            return Landing(out, file.absolutePath, context, null, file)
        }
    }
}
