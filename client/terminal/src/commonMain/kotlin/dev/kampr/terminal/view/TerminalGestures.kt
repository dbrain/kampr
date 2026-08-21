package dev.kampr.terminal.view

import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.awaitLongPressOrCancellation
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.input.pointer.PointerEvent
import androidx.compose.ui.input.pointer.PointerInputScope
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.input.pointer.util.VelocityTracker
import dev.kampr.terminal.PaneSession
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
    onTap: (Offset) -> Unit,
) {
    val view = session.view
    awaitEachGesture {
        val tracker = VelocityTracker()
        val down = awaitFirstDown(requireUnconsumed = false)
        tracker.addPosition(down.uptimeMillis, down.position)
        view.velocityX = 0f
        view.velocityY = 0f

        val held = awaitLongPressOrCancellation(down.id)
        if (held != null) {
            val anchor = probe.cellAt(held.position)
            view.selection = Selection(anchor, anchor, view.blockSelect)
            view.target = null
            var event: PointerEvent
            do {
                event = awaitPointerEvent()
                val moving = event.changes.firstOrNull { it.pressed } ?: break
                view.selection = view.selection?.copy(head = probe.cellAt(moving.position))
                event.changes.forEach { if (it.positionChanged()) it.consume() }
            } while (event.changes.any { it.pressed })
            return@awaitEachGesture
        }

        // awaitLongPressOrCancellation consumes the release, so a quick tap ends the gesture here
        // and never reaches the pan loop below — waiting for another event would hang it.
        if (currentEvent.changes.none { it.pressed }) {
            onTap(down.position)
            session.reclaimKeyboard()
            return@awaitEachGesture
        }

        var moved = false
        var multi = false
        var event: PointerEvent
        do {
            event = awaitPointerEvent()
            if (event.changes.any { it.isConsumed }) break
            val pressed = event.changes.filter { it.pressed }
            val pan = event.calculatePan()
            if (pressed.size > 1) {
                multi = true
                val centroid = event.calculateCentroid(useCurrent = true)
                view.pinch(centroid.x, centroid.y, pan.x, pan.y, event.calculateZoom())
                moved = true
            } else if (pan != Offset.Zero) {
                view.panX = (view.panX + pan.x).coerceIn(view.minPanX, 0f)
                view.scrollY = (view.scrollY - pan.y).coerceIn(0f, view.maxScroll)
                if (abs(pan.x) + abs(pan.y) > 1f) moved = true
            }
            pressed.firstOrNull()?.let { tracker.addPosition(it.uptimeMillis, it.position) }
            event.changes.forEach { if (it.positionChanged()) it.consume() }
        } while (event.changes.any { it.pressed })

        view.settle(presets, paint.contentBottom)
        when {
            !moved && !multi -> onTap(down.position)
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
