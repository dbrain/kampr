package dev.kampr.shared.platform

import kotlinx.browser.window

actual fun reduceMotionSetting(): Boolean =
    runCatching { window.matchMedia("(prefers-reduced-motion: reduce)").matches }.getOrDefault(false)
