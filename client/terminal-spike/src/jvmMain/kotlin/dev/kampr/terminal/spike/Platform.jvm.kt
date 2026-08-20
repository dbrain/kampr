package dev.kampr.terminal.spike

actual fun emitBench(line: String) {
    println(line)
}

actual val platformLabel: String =
    "jvm-desktop/" + System.getProperty("os.name") + "/" + System.getProperty("java.version")

actual fun graphicsBackend(): String =
    "skiko/" + (System.getProperty("skiko.renderApi") ?: "auto")
