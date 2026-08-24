package dev.kampr.shared.platform

import java.io.File

actual fun saveToDevice(name: String, mime: String?, bytes: ByteArray): String? = runCatching {
    val home = File(System.getProperty("user.home") ?: return@runCatching null)
    val dir = File(home, "Downloads").takeIf { it.isDirectory } ?: home
    val file = unclaimed(dir, name)
    file.writeBytes(bytes)
    file.path
}.getOrNull()

private fun unclaimed(dir: File, name: String): File {
    val file = File(dir, name)
    if (!file.exists()) return file
    val stem = name.substringBeforeLast('.')
    val suffix = name.substringAfterLast('.', "").let { if (it.isEmpty()) "" else ".$it" }
    var n = 2
    while (File(dir, "$stem-$n$suffix").exists()) n++
    return File(dir, "$stem-$n$suffix")
}
