package dev.kampr.terminal.bench

actual fun emitBench(line: String) = println(line)

actual val platformLabel: String =
    "jvm/" + System.getProperty("os.name") + "/" + System.getProperty("java.version")

actual fun graphicsBackend(): String = "skiko/" + (System.getProperty("skiko.renderApi") ?: "default")
