package dev.kampr.mosaic

import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import dev.kampr.shared.model.Herd
import dev.kampr.shared.model.closed
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.platform.Prefs
import dev.kampr.shared.wire.PaneInfo

private const val KEY_ARRANGEMENT = "mosaic.panes"
private const val KEY_PAIRED = "mosaic.paired"
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

    // Only ever set by `arrange`, and let go the moment the set of panes changes: it is the shape
    // herdr gave one split, not a preference about the mosaic, so it is never saved either.
    var stacked: Boolean by mutableStateOf(false)
        private set

    // A phone's choice to look at two panes at once. Unlike the arrangement it needs no Save: it
    // is how this device is held, not a layout anyone would want to keep two of.
    var paired: Boolean by mutableStateOf(false)
        private set

    // The two halves in screen order, so a tap on a chip never moves the cell being typed into
    // from one half to the other.
    private var halves: List<String> by mutableStateOf(emptyList())

    val shown: List<String>
        get() = if (paired) halves else listOfNotNull(focused)

    // Every pane known to have been in a herd, with what it was last seen as. Only one of these can
    // have closed: a pane missing that was never seen is still arriving.
    private val seen = HashMap<String, PaneInfo?>()

    private var attached = false

    // Snapshot state, not a read straight off disk: the Save control has to stop saying "save"
    // the moment it has, and Prefs is invisible to recomposition.
    private var persisted by mutableStateOf("")

    val saved: Boolean get() = encodeArrangement(panes) == persisted

    val full: Boolean get() = panes.size >= MAX_CELLS

    fun restore() {
        persisted = prefs.get(KEY_ARRANGEMENT).orEmpty()
        panes = decodeArrangement(persisted)
        paired = prefs.get(KEY_PAIRED) == "1"
        stacked = false
        // A saved layout was made from panes in a herd, so they count as seen: one that closed
        // while the app was away goes on the first herd that can say so.
        for (pane in panes) seen.getOrPut(pane) { null }
        halves = emptyList()
        land(panes.firstOrNull())
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

    fun arrange(next: List<String>, stacked: Boolean) {
        val kept = next.distinct().take(MAX_TAB_CELLS)
        if (attached) {
            for (gone in panes - kept.toSet()) connection.unwatch(gone, OWNER)
            for (added in kept - panes.toSet()) connection.watch(added, OWNER)
        }
        panes = kept
        this.stacked = stacked
        halves = emptyList()
        land(kept.lastOrNull())
    }

    fun add(paneId: String) {
        if (paneId in panes || full) return
        stacked = false
        panes = panes + paneId
        if (attached) connection.watch(paneId, OWNER)
        land(paneId)
    }

    fun remove(paneId: String) {
        if (paneId !in panes) return
        stacked = false
        drop(listOf(paneId))
    }

    // A chip already on screen only moves the focus; one that is not takes the half the focus is
    // not in, so the pane being typed into stays where it is.
    fun focus(paneId: String) {
        if (paneId in panes) land(paneId)
    }

    fun pair(on: Boolean) {
        paired = on
        prefs.set(KEY_PAIRED, if (on) "1" else null)
        halves = emptyList()
        land(focused)
    }

    private fun land(next: String?) {
        val previous = focused
        focused = next
        halves = if (paired && next != null) halvesFor(next, previous) else emptyList()
    }

    private fun halvesFor(next: String, previous: String?): List<String> {
        val slots = halves.ifEmpty { listOf(null, null) }.map { id -> id?.takeIf { it in panes } }.toMutableList()
        if (next !in slots) slots[slots.indexOf(null).takeIf { it >= 0 } ?: slots.indexOfFirst { it != previous }] = next
        val empty = slots.indexOf(null)
        if (empty >= 0) slots[empty] = neighbour(next) ?: return listOf(next)
        return slots.filterNotNull()
    }

    private fun neighbour(paneId: String): String? {
        val at = panes.indexOf(paneId)
        return panes.getOrNull(at + 1) ?: panes.getOrNull(at - 1)
    }

    // The focus goes to the other half when there is one on screen, and otherwise to whatever took
    // the gone pane's place in the order, or the pane before it when it was last.
    private fun drop(gone: List<String>) {
        if (attached) for (id in gone) connection.unwatch(id, OWNER)
        val before = panes
        panes = panes - gone.toSet()
        if (focused in panes) return land(focused)
        val at = before.indexOf(focused)
        land(
            halves.firstOrNull { it in panes }
                ?: before.drop(at + 1).firstOrNull { it in panes }
                ?: before.take(at.coerceAtLeast(0)).lastOrNull { it in panes },
        )
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
        focus(panes[((at + delta) % panes.size + panes.size) % panes.size])
    }

    fun save() {
        persisted = encodeArrangement(panes)
        prefs.set(KEY_ARRANGEMENT, persisted)
    }

    // Only a shell that exited takes its cell. A node that left may come back with the pane still on
    // it, and a node whose herdr is down drops every pane from the herd while it is still there,
    // which is why the node has to be serving before an absence means anything.
    fun reconcile(herd: Herd) {
        if (!herd.known) return
        for (pane in herd.panes) if (pane.id in panes) seen[pane.id] = pane
        val closed = panes.filter { id -> herd.closed(id, id in seen, seen[id]) }
        if (closed.isEmpty()) return
        for (id in closed) seen.remove(id)
        val saved = decodeArrangement(persisted)
        val kept = saved - closed.toSet()
        if (kept.size != saved.size) {
            persisted = encodeArrangement(kept)
            prefs.set(KEY_ARRANGEMENT, persisted)
        }
        drop(closed)
    }

    // One `observe` stream per cell, which is the whole cost of looking at four panes at once.
    val observers: Int get() = if (attached) panes.size else 0
}
