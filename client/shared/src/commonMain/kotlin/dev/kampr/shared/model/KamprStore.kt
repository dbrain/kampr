package dev.kampr.shared.model

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshots.Snapshot
import dev.kampr.shared.wire.PanePrefs
import dev.kampr.shared.wire.PaneInfo
import dev.kampr.shared.wire.Security
import dev.kampr.shared.wire.ServerMsg
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

sealed interface ConnectionStatus {
    data object Idle : ConnectionStatus
    data object Connecting : ConnectionStatus
    data class Live(val role: String) : ConnectionStatus
    data class Offline(val reason: String, val retryInMs: Long) : ConnectionStatus

    // The node is up and will not have this device. Separate from `Offline` because they are not
    // the same news: one comes back on its own and one never will until somebody pairs again.
    data class Refused(val reason: String) : ConnectionStatus
}

class KamprStore {
    private val _status = MutableStateFlow<ConnectionStatus>(ConnectionStatus.Idle)
    val status: StateFlow<ConnectionStatus> = _status.asStateFlow()

    private val _hello = MutableStateFlow<ServerMsg.Hello?>(null)
    val hello: StateFlow<ServerMsg.Hello?> = _hello.asStateFlow()

    private val _herd = MutableStateFlow(Herd())
    val herd: StateFlow<Herd> = _herd.asStateFlow()

    private val _failure = MutableStateFlow<ServerMsg.Failure?>(null)
    val failure: StateFlow<ServerMsg.Failure?> = _failure.asStateFlow()

    private val _localRttMs = MutableStateFlow<Double?>(null)
    val localRttMs: StateFlow<Double?> = _localRttMs.asStateFlow()

    private val _prefs = MutableStateFlow<Map<String, PanePrefs>>(emptyMap())
    val prefs: StateFlow<Map<String, PanePrefs>> = _prefs.asStateFlow()

    private val _nodeCaps = MutableStateFlow<Map<String, ServerMsg.NodeCaps>>(emptyMap())
    val nodeCaps: StateFlow<Map<String, ServerMsg.NodeCaps>> = _nodeCaps.asStateFlow()

    private val _managed = MutableStateFlow<ServerMsg.Managed?>(null)
    val managed: StateFlow<ServerMsg.Managed?> = _managed.asStateFlow()

    private val paneStates = mutableStateMapOf<String, PaneState>()
    val styles = StyleTable()

    // The role moves under a live socket — the node sends `role` on any mid-session demotion or
    // promotion — so this is snapshot state rather than a field of `hello` read once at connect.
    // Every affordance that gates on it recomposes when it moves; a StateFlow read would not.
    // `hello` itself stays the greeting this connection was given.
    var role: String by mutableStateOf("full")
        private set

    // What the operator is told. A device that silently loses its buttons is nearly as bad as one
    // that keeps buttons which no longer work.
    var roleNote: String? by mutableStateOf(null)
        private set

    val readOnly: Boolean get() = role == "readonly"

    // A node that does not implement the ops says so, and a read-only device is refused every one
    // of them with not_writer — either way the affordances must be absent, not present-and-failing.
    val canManage: Boolean get() = _hello.value?.caps?.manage == true && !readOnly

    val security: Security get() = _hello.value?.security ?: Security()

    fun capsFor(nodeId: String?): ServerMsg.NodeCaps? = capsFor(nodeId, _nodeCaps.value, _herd.value)

    fun clearManaged() {
        _managed.value = null
    }

    fun prefsFor(paneId: String): PanePrefs = _prefs.value[paneId] ?: PanePrefs()

    fun dismissFailure() {
        _failure.value?.pane?.let { paneStates[it]?.clearRefusal() }
        _failure.value = null
    }

    fun dismissRoleNote() {
        roleNote = null
    }

    fun pane(id: String): PaneState = paneStates.getOrPut(id) { PaneState(id, styles) }

    fun paneInfo(id: String): PaneInfo? = _herd.value.panes.firstOrNull { it.id == id }

    fun blocked(): List<PaneInfo> = _herd.value.panes.filter { statusOf(it) == AgentStatus.Blocked }

    // The triage list: every blocked agent with whatever question the node has published for it,
    // newest movement first. This is what the roadmap called the one Collie product idea worth
    // stealing wholesale, and what a notification tap lands on when more than one agent is waiting.
    //
    // Reads pane state without creating any: asking `pane(id)` here would write into a snapshot
    // state map during composition, which is a recomposition loop rather than a lookup.
    fun triage(): List<TriageItem> = blocked()
        .sortedByDescending { it.updatedAt ?: "" }
        .map { TriageItem(it, paneStates[it.id]?.pending?.question) }

    fun status(value: ConnectionStatus) {
        _status.value = value
    }

    fun recordRtt(ms: Double) {
        _localRttMs.value = ms
    }

    // Only for a pane that is actually on screen: creating one here would announce a lost
    // keystroke against a pane nobody is looking at.
    fun noteInput(paneId: String, delivered: Boolean) {
        val pane = paneStates[paneId] ?: return
        if (delivered) pane.noteDelivered() else pane.noteUndelivered()
    }

    // The node's socket plane reporting a pane doing things is only evidence about that pane's
    // stream if this client is actually watching it — so it is counted against the panes there is
    // a `PaneState` for, which is the set that has a grid on screen to be wrong. Creating one here
    // would accuse a pane nobody has open.
    private fun noteMovement(before: Herd) {
        if (paneStates.isEmpty()) return
        val was = before.panes.associateBy { it.id }
        for (after in _herd.value.panes) {
            val pane = paneStates[after.id] ?: continue
            val old = was[after.id] ?: continue
            if (paneMoved(old, after)) pane.noteMoved()
        }
    }

    fun markStale() {
        _herd.value = _herd.value.copy(stale = true)
        paneStates.values.forEach { it.markStale() }
    }

    // `stream_unavailable` is the one refusal nobody did anything to earn, and the one that has an
    // end nothing else announces: the node has no frame meaning "never mind", so the herd entry
    // clearing is the recovery signal. Every other code stays until it is dismissed, because every
    // other code is an answer to something the operator asked for.
    private fun dropRepairedFault() {
        val failure = _failure.value ?: return
        if (failure.code != "stream_unavailable") return
        val pane = failure.pane ?: return
        if (paneInfo(pane)?.detail == null) _failure.value = null
    }

    fun accept(msg: ServerMsg) {
        when (msg) {
            is ServerMsg.Hello -> {
                _hello.value = msg
                Snapshot.withMutableSnapshot {
                    role = msg.role
                    roleNote = null
                }
                _status.value = ConnectionStatus.Live(msg.role)
            }
            is ServerMsg.RoleChanged -> {
                // One change, not two. Written a statement apart, a reader watching for the role
                // to move landed between them 31% of the time and saw the new role with no notice
                // beside it — which is a demotion that took the operator's buttons away and never
                // said why.
                Snapshot.withMutableSnapshot {
                    if (msg.role != role) {
                        role = msg.role
                        roleNote = roleNoteFor(msg.role)
                    }
                }
                _status.value = ConnectionStatus.Live(msg.role)
            }
            is ServerMsg.Herd -> {
                val before = _herd.value
                _herd.value = Herd(msg.nodes, msg.panes, stale = false, known = true)
                noteMovement(before)
                dropRepairedFault()
            }
            is ServerMsg.HerdPatch -> {
                val before = _herd.value
                _herd.value = _herd.value.applyPatch(msg)
                noteMovement(before)
                dropRepairedFault()
            }
            is ServerMsg.Styles -> styles.append(msg.from, msg.styles)
            is ServerMsg.GridReset -> pane(msg.pane).applyReset(msg)
            is ServerMsg.GridPatch -> pane(msg.pane).applyPatch(msg)
            is ServerMsg.Scrollback -> pane(msg.pane).applyScrollback(msg)
            // A page merges by id, which is what lets `convo.load` prepend older slices of the
            // same transcript — and what leaves a conversation the pane has left sitting under
            // the one it moved to. `fresh` is the node saying this page is a different
            // transcript in a case where it cannot name the turns to withdraw: a socket it has
            // never seen, because this client reconnected or that node restarted.
            is ServerMsg.Convo -> pane(msg.pane).let { pane ->
                // A page naming a sub is a conversation the pane's agent *launched*, and it is
                // the one page that must never reach `turns`: those ids belong to another
                // transcript, and a reader would be shown a subagent's words as this pane's own
                // reply. `fresh` is set on every one of them, so routing it here would clear the
                // pane's transcript and replace it outright.
                if (msg.sub != null) {
                    pane.applySubConvo(msg)
                } else {
                    if (msg.fresh) pane.turns.clear()
                    pane.applyConvo(msg)
                }
            }
            is ServerMsg.ConvoTurn -> pane(msg.pane).applyConvoTurn(msg)
            is ServerMsg.Pending -> pane(msg.pane).pending = msg.takeIf { it.question != null }
            is ServerMsg.Prefs -> _prefs.value = _prefs.value + msg.panes
            is ServerMsg.Failure -> {
                _failure.value = msg
                // Read without creating: a refusal about a pane nobody has open is not news about
                // any grid on screen.
                msg.pane?.let { paneStates[it]?.refusal = msg.message }
                if (msg.code == "stream_unavailable") {
                    msg.pane?.let { paneStates[it]?.noteStreamStopped() }
                }
            }
            is ServerMsg.Managed -> _managed.value = msg
            is ServerMsg.NodeCaps -> _nodeCaps.value = _nodeCaps.value + (msg.node to msg)
            is ServerMsg.Pong -> Unit
        }
    }
}

// The node answers `caps` for itself only, and a named session is its own herdr server joining
// the same herd as a node of its own — so one host's reply is the answer for every session it
// runs, matched by the host in the node's *name* because an id is opaque and never a prefix. What
// this must not do is answer for a machine that has not replied: the map holds one entry for as
// long as it takes a hub's peers to answer, and borrowing that one handed a pane on peer B the
// affordances of peer A.
fun capsFor(
    nodeId: String?,
    caps: Map<String, ServerMsg.NodeCaps>,
    herd: Herd,
): ServerMsg.NodeCaps? {
    if (nodeId == null) return null
    caps[nodeId]?.let { return it }
    val host = herd.nodes.firstOrNull { it.id == nodeId }?.host ?: return null
    return caps.values.firstOrNull { answer ->
        herd.nodes.firstOrNull { it.id == answer.node }?.host == host
    }
}

private fun roleNoteFor(role: String): String = when (role) {
    "readonly" -> "This device is now read-only. It still sees every pane; it can no longer type into one."
    else -> "This device can write again. Typing and pane actions are back."
}
