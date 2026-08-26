package dev.kampr.shared.platform

import androidx.compose.runtime.Composable
import androidx.compose.runtime.ProvidableCompositionLocal
import androidx.compose.runtime.staticCompositionLocalOf

// Three answers collapsed onto two, and deliberately onto the side that costs least. "No keyboard"
// and "this platform cannot tell" are the same value here, because the two mistakes are not the
// same size: a key row shown to someone with a keyboard is a strip of clutter along the bottom of
// a window that has room for it, and a key row hidden from a tablet that has none leaves an
// operator with no Escape, no Ctrl and no arrows — no way out of vim and no way to answer a prompt.
//
// So every platform's uncertainty falls to `false`, and only a positive reading returns `true`.
@Composable
expect fun hardKeyboardAttached(): Boolean

// Provided once at the app root from `hardKeyboardAttached()`, so that a layout reads a value and
// a test can provide one — which is the only way this suite can see the case the report is about,
// since the JVM actual is a constant.
val LocalHardKeyboard: ProvidableCompositionLocal<Boolean> = staticCompositionLocalOf { false }
