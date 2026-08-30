package dev.kampr.shared.platform

import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Modifier

// What the operator chose to hand the agent. `name` is a hint at the stem and nothing more — the
// node owns the directory it writes to and derives the extension from the bytes, because an
// extension the sender chose is an extension the sender chose.
class PickedFile(val name: String?, val mime: String?, val bytes: ByteArray)

// Absent rather than present-and-failing: a platform with no picker draws no attach button.
expect val filePickAvailable: Boolean

// Null is "nothing was chosen", which is what backing out of a system picker means and is not a
// failure to report.
expect suspend fun pickFile(): PickedFile?

// A file the operator *pasted*, which is a different gesture from the one the attach button
// serves and — in a browser, where the clipboard is the only path between a screenshot tool and
// this page — the one that was missing. Suspends until one arrives; null means nothing more will,
// which is what a platform with no paste event of its own says immediately.
expect suspend fun pastedFile(): PickedFile?

// Watches for as long as it is composed and no longer, so a paste is never handled twice and
// never lands on a pane that has gone off screen. Files arrive one at a time: several in one
// clipboard are several calls, in the order the platform listed them.
@Composable
fun PastedFiles(enabled: Boolean, onFile: suspend (PickedFile) -> Unit) {
    // Keyed on `enabled` and nothing else. A live pane recomposes on every frame it paints and the
    // handler is a fresh lambda each time, so keying the effect on it would cancel the waiter
    // before any clipboard could reach it.
    val handler = rememberUpdatedState(onFile)
    LaunchedEffect(enabled) {
        if (!enabled) return@LaunchedEffect
        while (true) handler.value(pastedFile() ?: break)
    }
}

// Where a pasted file is allowed to land. **Android has no page-wide paste event**: a clipboard
// image reaches an app only through the text field the operator pasted into, which is why this is
// a modifier rather than a listener — and only through a `BasicTextField` built on a
// `TextFieldState`, because the older one refuses `commitContent` outright and declares no content
// types for a keyboard to offer (#368). Everywhere else the browser or the desktop already decides
// where a paste goes, and this is the identity it should be.
@Composable
expect fun Modifier.acceptsPastedFiles(): Modifier
