package dev.kampr.shared.model

import androidx.compose.runtime.mutableStateMapOf
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

    val readOnly: Boolean get() = _hello.value?.role == "readonly"

    // A node that does not implement the ops says so, and a read-only device is refused every one
    // of them with not_writer — either way the affordances must be absent, not present-and-failing.
    val canManage: Boolean get() = _hello.value?.caps?.manage == true && !readOnly

    val security: Security get() = _hello.value?.security ?: Security()

    // The node answers `caps` for itself only, so a herd whose nodes are named sessions of one
    // host has a single reply covering all of them. Falling back to it beats parsing an opaque id.
    fun capsFor(nodeId: String?): ServerMsg.NodeCaps? =
        _nodeCaps.value[nodeId] ?: _nodeCaps.value.values.singleOrNull()

    fun clearManaged() {
        _managed.value = null
    }

    fun prefsFor(paneId: String): PanePrefs = _prefs.value[paneId] ?: PanePrefs()

    fun dismissFailure() {
        _failure.value = null
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

    fun markStale() {
        _herd.value = _herd.value.copy(stale = true)
        paneStates.values.forEach { it.markStale() }
    }

    fun accept(msg: ServerMsg) {
        when (msg) {
            is ServerMsg.Hello -> {
                _hello.value = msg
                _status.value = ConnectionStatus.Live(msg.role)
            }
            is ServerMsg.Herd -> _herd.value = Herd(msg.nodes, msg.panes, stale = false, known = true)
            is ServerMsg.HerdPatch -> _herd.value = _herd.value.applyPatch(msg)
            is ServerMsg.Styles -> styles.append(msg.from, msg.styles)
            is ServerMsg.GridReset -> pane(msg.pane).applyReset(msg)
            is ServerMsg.GridPatch -> pane(msg.pane).applyPatch(msg)
            is ServerMsg.Scrollback -> pane(msg.pane).applyScrollback(msg)
            is ServerMsg.Convo -> pane(msg.pane).applyConvo(msg)
            is ServerMsg.ConvoTurn -> pane(msg.pane).applyConvoTurn(msg)
            is ServerMsg.Pending -> pane(msg.pane).pending = msg.takeIf { it.question != null }
            is ServerMsg.Prefs -> _prefs.value = _prefs.value + msg.panes
            is ServerMsg.Failure -> _failure.value = msg
            is ServerMsg.Managed -> _managed.value = msg
            is ServerMsg.NodeCaps -> _nodeCaps.value = _nodeCaps.value + (msg.node to msg)
            is ServerMsg.Pong -> Unit
        }
    }
}
