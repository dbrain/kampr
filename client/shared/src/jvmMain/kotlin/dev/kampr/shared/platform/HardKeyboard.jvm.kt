package dev.kampr.shared.platform

import androidx.compose.runtime.Composable

// A desktop window is opened by someone sitting at a machine that has a keyboard on it. There is
// no AWT or JVM reading of whether one is attached — `Toolkit` reports the lock states of keys it
// assumes exist — and a headless desktop with no keyboard is not a thing this app is run on.
@Composable
actual fun hardKeyboardAttached(): Boolean = true
