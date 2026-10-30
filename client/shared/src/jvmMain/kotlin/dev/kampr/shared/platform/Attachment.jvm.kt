package dev.kampr.shared.platform

import dev.kampr.shared.net.DownloadSink
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import java.io.File

actual fun saveToDevice(name: String, mime: String?, bytes: ByteArray): String? = runCatching {
    val file = landing(name) ?: return@runCatching null
    file.writeBytes(bytes)
    file.path
}.getOrNull()

actual fun openDownload(name: String, mime: String?): DownloadSink? {
    val file = landing(name) ?: return null
    val out = runCatching { file.outputStream() }.getOrNull() ?: return null
    return object : DownloadSink {
        override suspend fun write(chunk: ByteArray, length: Int) = withContext(Dispatchers.IO) { out.write(chunk, 0, length) }
        override suspend fun finish(): String? = withContext(Dispatchers.IO) { runCatching { out.close() }.map { file.path }.getOrNull() }
        override suspend fun abort() = withContext(Dispatchers.IO) {
            runCatching { out.close() }
            file.delete()
            Unit
        }
    }
}

private fun landing(name: String): File? {
    val home = File(System.getProperty("user.home") ?: return null)
    val dir = File(home, "Downloads").takeIf { it.isDirectory } ?: home
    return unclaimed(dir, name)
}

private fun unclaimed(dir: File, name: String): File {
    val file = File(dir, name)
    if (!file.exists()) return file
    val stem = name.substringBeforeLast('.')
    val suffix = name.substringAfterLast('.', "").let { if (it.isEmpty()) "" else ".$it" }
    var n = 2
    while (File(dir, "$stem-$n$suffix").exists()) n++
    return File(dir, "$stem-$n$suffix")
}
