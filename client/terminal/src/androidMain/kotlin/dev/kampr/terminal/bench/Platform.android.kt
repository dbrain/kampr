package dev.kampr.terminal.bench

import android.os.Build
import android.util.Log

actual fun emitBench(line: String) {
    Log.i("KamprBench", line)
}

actual val platformLabel: String =
    "android/" + Build.VERSION.SDK_INT + "/" + Build.MODEL + "/" + Build.SUPPORTED_ABIS.firstOrNull()

actual fun graphicsBackend(): String = "hwui/" + Build.HARDWARE
