package dev.kampr.shared.platform

import android.provider.Settings

// The animation scales are what Chrome on Android reports as prefers-reduced-motion, and what
// "Remove animations" in accessibility settings drives to zero.
actual fun reduceMotionSetting(): Boolean {
    val resolver = KamprAndroid.context?.contentResolver ?: return false
    fun scale(key: String) = Settings.Global.getFloat(resolver, key, 1f)
    return runCatching {
        scale(Settings.Global.ANIMATOR_DURATION_SCALE) == 0f ||
            scale(Settings.Global.TRANSITION_ANIMATION_SCALE) == 0f
    }.getOrDefault(false)
}
