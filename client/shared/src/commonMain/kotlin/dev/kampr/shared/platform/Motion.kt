package dev.kampr.shared.platform

import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.staticCompositionLocalOf

// Whether this device asks for less movement: Android's animator scale, the browser's
// prefers-reduced-motion, GNOME's Gtk/EnableAnimations. Read once — a running app does not
// resample it, and nothing in Kampr animates for long enough to care.
expect fun reduceMotionSetting(): Boolean

private val platformSetting: Boolean by lazy { reduceMotionSetting() }

val LocalReduceMotion: ProvidableCompositionLocal<Boolean> = staticCompositionLocalOf { platformSetting }
