package dev.kampr.shared.ui

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitLongPressOrCancellation
import androidx.compose.runtime.Composable
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.pointer.PointerEventPass
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.isSecondaryPressed
import androidx.compose.ui.input.pointer.pointerInput

// Herdr opens its own menus on a right-click, so Kampr opens the same sheet on the same
// gesture and an operator who knows one knows the other. A finger has no second button, so
// a long press is the touch spelling of it — taken only where the surface is not already
// spending one: the terminal grid's long press starts a selection and the mosaic strip's
// reorders the chips, and neither may be taken away.
//
// This is a shortcut, never the only way in: the same sheet is behind the "…" glyph on
// every surface that carries this. That is what lets the gesture itself stay silent to a
// screen reader, which reaches the sheet through the caller's own `Modifier.action`.
@Composable
fun Modifier.paneActions(paneId: String, longPress: Boolean = true): Modifier {
    val manage = LocalManage.current
    if (!manage.enabled) return this
    return this.pointerInput(paneId, longPress) {
        awaitEachGesture {
            // Initial, because a card is a button and a pane is a grid: both would
            // otherwise take the press before the second button is ever looked at.
            val down = awaitFirstDown(requireUnconsumed = false, pass = PointerEventPass.Initial)
            if (currentEvent.buttons.isSecondaryPressed) {
                down.consume()
                manage.openActions(paneId)
                return@awaitEachGesture
            }
            if (longPress && down.type == PointerType.Touch) {
                awaitLongPressOrCancellation(down.id)?.let {
                    it.consume()
                    manage.openActions(paneId)
                }
            }
        }
    }
}
