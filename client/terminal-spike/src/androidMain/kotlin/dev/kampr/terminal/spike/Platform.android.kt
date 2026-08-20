package dev.kampr.terminal.spike

import android.os.Build
import android.util.Log

actual fun emitBench(line: String) {
    Log.i("KamprSpike", line)
}

actual val platformLabel: String =
    "android-api${Build.VERSION.SDK_INT}/${Build.MODEL}/${Build.HARDWARE}"

actual fun graphicsBackend(): String =
    "android-hwui-skia/vulkan-or-gles (renderer chosen by platform); " +
        "board=${Build.BOARD} device=${Build.DEVICE} fingerprint=${Build.FINGERPRINT}"
