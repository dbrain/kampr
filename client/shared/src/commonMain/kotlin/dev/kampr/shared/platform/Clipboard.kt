package dev.kampr.shared.platform

import androidx.compose.runtime.staticCompositionLocalOf

// Reading text off the clipboard is the one clipboard direction Compose has no common answer for.
// `ClipboardManager.getText` is deprecated and, on the web, returns null by construction — the
// browser's clipboard is async, so the synchronous actual there is a hard-coded null. `Clipboard`
// replaces it, but its `ClipEntry` is an `expect class` whose every accessor is platform-native, so
// there is nothing common to read a string out of. Hence three actuals of our own.
//
// Null is "there is no text on the clipboard", which is not a failure — but it is also what a
// refused browser permission looks like, so a caller must say so on screen rather than doing
// nothing and looking broken.
expect suspend fun clipboardText(): String?

// A local rather than a direct call, because no test can rely on a system clipboard: the desktop's
// is AWT's and absent on a headless runner, and the browser's is behind a permission prompt.
val LocalClipboardText = staticCompositionLocalOf<suspend () -> String?> { { clipboardText() } }
