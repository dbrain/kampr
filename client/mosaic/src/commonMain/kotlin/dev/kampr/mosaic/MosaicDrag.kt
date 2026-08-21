package dev.kampr.mosaic

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue

private data class CellRect(val left: Float, val top: Float, val right: Float, val bottom: Float) {
    fun holds(x: Float, y: Float): Boolean = x >= left && x < right && y >= top && y < bottom
}

// Where each cell actually ended up, in window coordinates. The shape is computed from the count
// and the window, so nothing else in the mosaic knows where a cell is — and a drag has to.
@Stable
class MosaicDrag {
    private val rects = HashMap<String, CellRect>()

    var held: String? by mutableStateOf(null)
        private set

    fun place(paneId: String, left: Float, top: Float, right: Float, bottom: Float) {
        rects[paneId] = CellRect(left, top, right, bottom)
    }

    fun forget(paneId: String) {
        rects.remove(paneId)
    }

    fun at(x: Float, y: Float): String? = rects.entries.firstOrNull { it.value.holds(x, y) }?.key

    fun start(paneId: String) {
        held = paneId
    }

    fun drag(x: Float, y: Float): String? = at(x, y)

    fun end() {
        held = null
    }
}
