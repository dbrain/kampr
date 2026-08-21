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

    // Snapshot state, not a read straight off disk: the Save control has to stop saying "save"
    // the moment it has, and Prefs is invisible to recomposition.
    private var persisted by mutableStateOf("")

    val saved: Boolean get() = encodeArrangement(panes) == persisted

    val full: Boolean get() = panes.size >= MAX_CELLS

    fun restore() {
        persisted = prefs.get(KEY_ARRANGEMENT).orEmpty()
        panes = decodeArrangement(persisted)
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

    // Order is layout and nothing else: no watch starts, none stops, and the focus travels with
    // the cell rather than with the position it used to occupy.
    fun move(paneId: String, toIndex: Int) {
        val from = panes.indexOf(paneId)
        if (from < 0) return
        val to = toIndex.coerceIn(0, panes.size - 1)
        if (from == to) return
        val next = panes.toMutableList()
        next.removeAt(from)
        next.add(to, paneId)
        panes = next
    }

    // One place at a time, and the ends stop rather than wrap. Wrapping is disorienting when the
    // grid it happens in cannot be seen, which is the path this exists for.
    fun moveBy(paneId: String, delta: Int): Boolean {
        val from = panes.indexOf(paneId)
        if (from < 0) return false
        val to = from + delta
        if (to !in panes.indices) return false
        move(paneId, to)
        return true
    }

    fun step(delta: Int) {
        if (panes.isEmpty()) return
        val at = panes.indexOf(focused).coerceAtLeast(0)
        focused = panes[((at + delta) % panes.size + panes.size) % panes.size]
    }

    fun save() {
        persisted = encodeArrangement(panes)
        prefs.set(KEY_ARRANGEMENT, persisted)
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
