package dev.kampr.terminal.view

import androidx.compose.ui.input.pointer.PointerEventType
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.isCtrlPressed
import androidx.compose.ui.input.pointer.isMetaPressed
import androidx.compose.ui.input.pointer.isShiftPressed
import kotlin.math.pow

// A notch is three rows, the way a terminal emulator moves.
internal const val WHEEL_ROWS = 3f

// One wheel click is one step of this much zoom. Small enough that a click is a nudge rather than
// a jump, since a wheel is the only continuous zoom a mouse has.
internal const val ZOOM_PER_CLICK = 1.1f

// The magnitude of `scrollDelta` belongs to the host, and the hosts disagree by two orders of
// magnitude: AWT and Android hand over wheel *clicks* — `1.0`, or a fraction of one from a precise
// trackpad — while CMP's web backend forwards the DOM wheel deltas as they arrive, around a
// hundred per notch in Chrome. That is why `androidx.compose.foundation` carries a per-platform
// `ScrollConfig` at all, and its web actual (`JsScrollable.web.kt`) converts with a plain `dp`
// factor where the desktop one goes through `MouseWheelEvent.getScrollAmount`. `ScrollConfig`
// needs a `CompositionLocalConsumerModifierNode` to reach, which a `pointerInput` block is not.
//
// So the sign and the arrival of an event are portable and the size of one is not: a delta below
// a click moves proportionally, and no single event moves more than one notch whatever number the
// host put in it. **Unverified on a real browser** — the web figure above is read off the shape of
// CMP's own web scroll config, not measured.
private fun notches(delta: Float) = (delta * WHEEL_ROWS).coerceIn(-WHEEL_ROWS, WHEEL_ROWS)

// The same rule `notches` applies, in clicks rather than rows: a fraction of a click is a fraction
// of a step, and no single event is worth more than one however large a number the host put in it.
// Negated because the wheel away from the reader — the direction that walks into history — is the
// direction every browser and every editor zooms in.
private fun zoomStep(delta: Float) = ZOOM_PER_CLICK.pow(-delta.coerceIn(-1f, 1f))

// The wheel goes through `TerminalViewState.scrollBy` — the same call the finger makes — so the
// clamps, the caret floor and `following` have exactly one implementation between them. A second
// path with its own arithmetic is how #175 and #177 happened.
//
// It is its own `pointerInput` because `terminalGestures` opens with `awaitFirstDown`, and a wheel
// never presses anything.
internal suspend fun PointerInputScope.terminalWheel(
    view: TerminalViewState,
    probe: GridProbe,
    presets: ZoomPresets,
) {
    awaitPointerEventScope {
        while (true) {
            val event = awaitPointerEvent()
            if (event.type != PointerEventType.Scroll) continue
            var dx = 0f
            var dy = 0f
            for (change in event.changes) {
                dx += change.scrollDelta.x
                dy += change.scrollDelta.y
                change.consume()
            }
            // The pinch in `terminalGestures` needs two *pressed* pointers, and a touchpad pinch
            // presses nothing — it arrives here, as a scroll with ctrl held, because that is what
            // a browser synthesises for one. So this single branch is the touchpad's pinch and the
            // mouse's only zoom at once, and a mouse had no zoom at all before it.
            if (event.keyboardModifiers.isCtrlPressed || event.keyboardModifiers.isMetaPressed) {
                if (dy != 0f) view.setZoom(view.zoom * zoomStep(dy), presets)
                continue
            }
            // A browser turns shift+wheel into its own horizontal axis; a desktop toolkit leaves
            // it on the vertical one. Reading both is what makes it the same gesture on both.
            if (event.keyboardModifiers.isShiftPressed && dx == 0f) {
                dx = dy
                dy = 0f
            }
            if (dx == 0f && dy == 0f) continue
            // Negated on both axes: the wheel says where the *content* goes, a drag says where the
            // surface goes, and `scrollBy` speaks the drag's language.
            view.scrollBy(-notches(dx) * probe.cellWidth, -notches(dy) * probe.cellHeight)
        }
    }
}
