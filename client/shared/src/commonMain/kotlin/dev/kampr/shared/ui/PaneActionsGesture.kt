package dev.kampr.shared.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.pointerInput

// Herdr opens its own menus on a right-click, so Kampr opens the same sheet on the same gesture
// and an operator who knows one knows the other. A finger has no second button, so the long press
// is the touch spelling of it — and that half belongs to `Modifier.action`, not to this: an inner
// `clickable` consumes the held press out from under an outer `awaitLongPressOrCancellation`, so
// the two composed to nothing on every surface that had both, which was all of them. A surface
// that is already spending its long press — the terminal grid selects, the mosaic strip reorders —
// opts out by passing no `onLongClick`; this half is the mouse's and every surface gets it.
//
// The raw event rather than `awaitFirstDown`, because on skiko — desktop and wasm, so every
// platform this client ships to — `isChangedToDown` refers a first down to the primary mouse
// button only and `awaitFirstDown` therefore never returns for a press carrying the second one.
// The event itself is honest: `type=Press sec=true pri=false`.
//
// Initial, because a card is a button and a pane is a grid: both would otherwise take the press
// before the second button is ever looked at.
//
// This is a shortcut, never the only way in: the same sheet is behind the "…" glyph on every
// surface that carries this. That is what lets the gesture itself stay silent to a screen reader,
// which reaches the sheet through the caller's own `Modifier.action`.
@Composable
fun Modifier.paneActions(paneId: String): Modifier {
    val manage = LocalManage.current
    if (!manage.enabled) return this
    return this.pointerInput(paneId) {
        awaitEachGesture {
            val event = awaitPointerEvent(PointerEventPass.Initial)
            if (event.type == PointerEventType.Press && event.buttons.isSecondaryPressed) {
                event.changes.forEach { it.consume() }
                manage.openActions(paneId)
            }
        }
    }
}
