package dev.kampr.terminal.view

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.AwaitPointerEventScope
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerEventTimeoutCancellationException
import androidx.compose.ui.input.pointer.PointerInputChange
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.PointerType
import androidx.compose.ui.input.pointer.positionChange
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.input.pointer.util.VelocityTracker
import dev.kampr.terminal.PaneSession
import dev.kampr.terminal.input.PaneScroll
import dev.kampr.terminal.render.Selection
import kotlin.math.abs

// One finger pans and flings straight into the committed origin, because pan needs no re-shaping.
// Two fingers drive the layer instead and only the settled zoom re-shapes (probe #60). A long
// press starts a selection, and a tap that never became a drag is what raises the keyboard —
// there is no button for it, the way a terminal emulator behaves.
internal suspend fun PointerInputScope.terminalGestures(
    session: PaneSession,
    presets: ZoomPresets,
    paint: PaintRect,
    probe: GridProbe,
    toPane: PaneScroll? = null,
    onTap: (Offset) -> Unit,
) {
    val view = session.view
    awaitEachGesture {
        val tracker = VelocityTracker()
        val down = awaitFirstDown(requireUnconsumed = false)
        tracker.addPosition(down.uptimeMillis, down.position)
        // Every scrolling surface takes a touch during a fling as a brake. This one took it as a
        // tap on the grid as well, so stopping a scroll you had overshot cost you the keyboard.
        val braking = abs(view.velocityX) > 1f || abs(view.velocityY) > 1f
        view.velocityX = 0f
        view.velocityY = 0f

        if (down.type == PointerType.Mouse) {
            mouseGesture(down, session, probe, braking, onTap)
            return@awaitEachGesture
        }

        val press = awaitStillPress(down)
        val held = press.held
        if (held != null) {
            val anchor = probe.cellAt(held.position)
            view.selection = Selection(anchor, anchor, view.blockSelect)
            view.aimOff()
            var event: PointerEvent
            do {
                event = awaitPointerEvent()
                val moving = event.changes.firstOrNull { it.pressed } ?: break
                view.selection = view.selection?.copy(head = probe.cellAt(moving.position))
                event.changes.forEach { if (it.positionChanged()) it.consume() }
            } while (event.changes.any { it.pressed })
            return@awaitEachGesture
        }

        // awaitStillPress consumes the release, so a quick tap ends the gesture here and never
        // reaches the pan loop below — waiting for another event would hang it.
        if (currentEvent.changes.none { it.pressed }) {
            if (press.travel <= viewConfiguration.touchSlop && !braking) onTap(down.position)
            session.reclaimKeyboard()
            return@awaitEachGesture
        }

        // What makes a gesture a scroll is how far the finger went, not how fast it was going at
        // any one moment — and the distance carries over from the press, which is where most of a
        // scroll is measured. Restarting the count here is what let a gesture already past the
        // slop end as a tap.
        var travel = press.travel
        var multi = false
        var event: PointerEvent
        toPane?.rest()
        do {
            event = awaitPointerEvent()
            if (event.changes.any { it.isConsumed }) break
            val pressed = event.changes.filter { it.pressed }
            val pan = event.calculatePan()
            if (pressed.size > 1) {
                multi = true
                val centroid = event.calculateCentroid(useCurrent = true)
                view.pinch(centroid.x, centroid.y, pan.x, pan.y, event.calculateZoom())
            } else if (pan != Offset.Zero) {
                val before = view.scrollY
                view.scrollBy(pan.x, pan.y)
                travel += abs(pan.x) + abs(pan.y)
                // The surface underneath is spent, so the rest of the drag belongs to whatever is
                // drawing the pane — the same hand-off the wheel makes, by distance rather than by
                // notch. Only a harness measured to take it is ever given one; `toPane` is null
                // everywhere else, which is every pane Kampr holds the history for itself.
                if (toPane != null && pan.y != 0f && view.scrollY == before) {
                    val cell = probe.cellAt(pressed.first().position)
                    toPane.refused(pan.y, probe.cellHeight, cell.col, cell.row)
                }
            }
            pressed.firstOrNull()?.let { tracker.addPosition(it.uptimeMillis, it.position) }
            event.changes.forEach { if (it.positionChanged()) it.consume() }
        } while (event.changes.any { it.pressed })

        view.settle(presets, paint.contentBottom)
        val dragged = travel > viewConfiguration.touchSlop
        when {
            !dragged && !multi -> if (!braking) onTap(down.position)
            !multi -> {
                val velocity = tracker.calculateVelocity()
                view.velocityX = velocity.x
                view.velocityY = velocity.y
                view.fling()
            }
        }
        session.reclaimKeyboard()
    }
}

// How far the finger went before the gesture was handed on, and whether it stayed still long
// enough to be a long press. The distance has to come back out: a gesture that leaves here past
// the touch slop is already a drag, and the pan loop that inherits it would otherwise start
// counting from zero.
private class StillPress(val held: PointerInputChange?, val travel: Float)

// A long press is a press that stayed still, and only that. Asking `awaitLongPressOrCancellation`
// alone asks how long the finger has been down and nothing about where it went — so a pinch, which
// is never over in the 500ms the platform allows, arrived as a selection and the zoom never moved.
// Travel past the touch slop, or a second finger, means the gesture belongs to the pan loop.
private suspend fun AwaitPointerEventScope.awaitStillPress(down: PointerInputChange): StillPress {
    var travel = 0f
    val held = try {
        withTimeout(viewConfiguration.longPressTimeoutMillis) {
            var last = down.position
            while (true) {
                val event = awaitPointerEvent()
                if (event.changes.count { it.pressed } > 1) break
                val change = event.changes.firstOrNull { it.id == down.id } ?: break
                // Counted before the release is examined, because a flick fast enough to arrive
                // as one move and an up carries its whole distance on the event that ends it.
                travel += (change.position - last).getDistance()
                last = change.position
                if (!change.pressed || travel > viewConfiguration.touchSlop) break
            }
            null
        }
    } catch (_: PointerEventTimeoutCancellationException) {
        down
    }
    return StillPress(held, travel)
}

// A mouse is not a fingertip, and a desk has no name for a press held still: it drags across the
// text it wants, the way every terminal emulator does. The drag is the selection's and nothing
// else's — the wheel already pans this surface on both axes and zooms it (`terminalWheel`), so a
// mouse loses nothing by giving it up. A press that never travelled is still a click, and a click
// is still the tap that puts a selection away and asks for the keyboard.
//
// The reading this is gated on is measured rather than assumed: a real mouse on the web build
// arrives as `PointerType.Mouse` (#382), which is the platform the report came from. The pointer
// is the question and the *input mode* is not — that flips to `Touch` mid-mouse-gesture there.
private suspend fun AwaitPointerEventScope.mouseGesture(
    down: PointerInputChange,
    session: PaneSession,
    probe: GridProbe,
    braking: Boolean,
    onTap: (Offset) -> Unit,
) {
    val view = session.view
    var travel = 0f
    var selecting = false
    var event: PointerEvent
    do {
        event = awaitPointerEvent()
        val moving = event.changes.firstOrNull { it.pressed }
        if (moving != null) {
            if (!selecting) {
                travel += moving.positionChange().getDistance()
                if (travel > viewConfiguration.touchSlop) {
                    val anchor = probe.cellAt(down.position)
                    view.selection = Selection(anchor, anchor, view.blockSelect)
                    view.aimOff()
                    selecting = true
                }
            }
            if (selecting) {
                view.selection = view.selection?.copy(head = probe.cellAt(moving.position))
                event.changes.forEach { if (it.positionChanged()) it.consume() }
            }
        }
    } while (event.changes.any { it.pressed })
    if (!selecting && !braking) onTap(down.position)
    session.reclaimKeyboard()
}
