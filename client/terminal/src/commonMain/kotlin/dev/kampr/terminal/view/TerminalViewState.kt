package dev.kampr.terminal.view

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.terminal.render.Selection
import dev.kampr.terminal.render.Target

// Probe #60: re-shaping at every intermediate zoom collapses the run cache to ~51% and drops 8.5%
// of frames at 200x50, so a pinch scales a layer and only the settled zoom re-shapes. Pan is a
// plain origin offset with no re-shaping, so it is applied straight to the committed state —
// putting it in the layer too would translate already-painted rows and leave the newly revealed
// ones blank until the finger lifts.
@Stable
class TerminalViewState {
    var zoom by mutableFloatStateOf(0f)
        private set
    var panX by mutableFloatStateOf(0f)
    var scrollY by mutableFloatStateOf(0f)

    var layerScale by mutableFloatStateOf(1f)
        private set
    var layerTx by mutableFloatStateOf(0f)
        private set
    var layerTy by mutableFloatStateOf(0f)
        private set

    var pinching by mutableStateOf(false)
        private set
    var sheetOpen by mutableStateOf(false)
    var selection by mutableStateOf<Selection?>(null)
    var blockSelect by mutableStateOf(false)
    var target by mutableStateOf<Target?>(null)
    var followCursor by mutableStateOf(true)
    var remembered by mutableStateOf(true)

    var velocityX = 0f
    var velocityY = 0f
    var minPanX = 0f
    var maxScroll = 0f

    var flings by mutableIntStateOf(0)
        private set

    fun fling() {
        flings++
    }

    val displayZoom: Float get() = zoom * layerScale

    var chosen = false
        private set

    // The default is re-derived until the operator picks a zoom of their own: history and the real
    // geometry both arrive after the first paint, and a zoom taken before them is wrong for good.
    fun adoptDefault(value: Float) {
        if (!chosen) zoom = value
    }

    // Pan and scroll are distances across the surface, not across the viewport, so a change of
    // cell size has to carry them or the viewport lands on a different row than the one being read.
    fun setZoom(value: Float, presets: ZoomPresets) {
        chosen = true
        val target = value.coerceIn(presets.minimum, presets.maximum)
        val applied = if (zoom > 0f) target / zoom else 1f
        panX *= applied
        scrollY *= applied
        zoom = target
    }

    fun drag(dx: Float, dy: Float) {
        panX += dx
        scrollY -= dy
    }

    fun pinch(centroidX: Float, centroidY: Float, panDx: Float, panDy: Float, scale: Float) {
        pinching = true
        layerScale *= scale
        layerTx = centroidX + (layerTx - centroidX) * scale + panDx
        layerTy = centroidY + (layerTy - centroidY) * scale + panDy
    }

    // Folding the layer back in: a surface point v lands at v*S + T, and origins are
    // originX = panX and originY = contentBottom - surfaceHeight + scrollY, so
    // panX' = panX*S + Tx and scrollY' = scrollY*S + Ty + contentBottom*(S - 1).
    fun settle(presets: ZoomPresets, contentBottom: Float) {
        if (!pinching) return
        pinching = false
        val scale = layerScale
        val tx = layerTx
        val ty = layerTy
        layerScale = 1f
        layerTx = 0f
        layerTy = 0f
        chosen = true
        val target = (zoom * scale).coerceIn(presets.minimum, presets.maximum)
        val applied = if (zoom > 0f) target / zoom else 1f
        panX = panX * applied + tx
        scrollY = scrollY * applied + ty + contentBottom * (applied - 1f)
        zoom = target
    }
}
