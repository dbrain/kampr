package dev.kampr.mosaic

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.model.Herd
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.platform.Prefs

private const val KEY_ARRANGEMENT = "mosaic.panes"
private const val OWNER = "mosaic"

// This is Kampr's own arrangement of watches over the merged herd, kept on this device. It is
// not herdr's `layout.export`, which is one server's split tree and reshapes real panes when it
// is applied; nothing here ever reaches a herdr server.
@Stable
class MosaicState(private val prefs: Prefs, private val connection: KamprConnection) {
    var panes: List<String> by mutableStateOf(emptyList())
        private set

    var focused: String? by mutableStateOf(null)
        private set

    private var attached = false

    val saved: Boolean get() = encodeArrangement(panes) == prefs.get(KEY_ARRANGEMENT).orEmpty()

    val full: Boolean get() = panes.size >= MAX_CELLS

    fun restore() {
        panes = decodeArrangement(prefs.get(KEY_ARRANGEMENT))
        focused = panes.firstOrNull()
    }

    // Nothing is observed while the mosaic is off screen: the streams belong to the cells that
    // are showing, and the arrangement outlives them.
    fun attach() {
        if (attached) return
        attached = true
        for (pane in panes) connection.watch(pane, OWNER)
    }

    fun detach() {
        if (!attached) return
        attached = false
        for (pane in panes) connection.unwatch(pane, OWNER)
    }

    fun add(paneId: String) {
        if (paneId in panes || full) return
        panes = panes + paneId
        if (attached) connection.watch(paneId, OWNER)
        focused = paneId
    }

    fun remove(paneId: String) {
        if (paneId !in panes) return
        panes = panes - paneId
        if (attached) connection.unwatch(paneId, OWNER)
        if (focused == paneId) focused = panes.firstOrNull()
    }

    fun focus(paneId: String) {
        if (paneId in panes) focused = paneId
    }

    fun step(delta: Int) {
        if (panes.isEmpty()) return
        val at = panes.indexOf(focused).coerceAtLeast(0)
        focused = panes[((at + delta) % panes.size + panes.size) % panes.size]
    }

    fun save() {
        prefs.set(KEY_ARRANGEMENT, encodeArrangement(panes))
    }

    // A pane that has left the herd cannot be watched and must not hold a cell hostage; the
    // arrangement on disk keeps it, because a peer that comes back brings its panes with it.
    fun reconcile(herd: Herd) {
        if (!herd.known) return
        val live = herd.panes.mapTo(HashSet()) { it.id }
        val kept = panes.filter { it in live }
        if (kept.size == panes.size) return
        if (attached) for (gone in panes - kept.toSet()) connection.unwatch(gone, OWNER)
        panes = kept
        if (focused !in kept) focused = kept.firstOrNull()
    }

    // One `observe` stream per cell, which is the whole cost of looking at four panes at once.
    val observers: Int get() = if (attached) panes.size else 0
}
