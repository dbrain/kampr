package dev.kampr.shared

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.platform.MemoryPrefs
import dev.kampr.shared.push.NoPush
import dev.kampr.shared.ui.AppState
import dev.kampr.shared.wire.Wire
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals

private const val PANE = "01JNODE/w1:p1"

private fun finished(at: String) = """{"t":"herd","nodes":[{"id":"01JNODE","name":"box","kind":"local"}],"panes":[""" +
    """{"id":"$PANE","node_id":"01JNODE","agent":"claude","agent_status":"done","updated_at":"$at"}]}"""

private fun StubNode.reads(): List<String> = received.filter { it.contains("\"done.read\"") }

// A finish read here has to be read everywhere: the phone's notification is the node's, and the
// node only knows what it is told. herdr's own `done` falls only when the desk focuses the pane,
// which Kampr may never do.
class DoneReadTest {
    private fun connected(node: StubNode, prefs: MemoryPrefs, body: suspend (AppState) -> Unit) {
        val scope = CoroutineScope(SupervisorJob())
        val state = AppState(scope, KamprStore(), prefs, null, NoPush())
        try {
            runBlocking {
                state.connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("the session") { state.store.status.value is ConnectionStatus.Live }
                body(state)
            }
        } finally {
            state.connection.disconnect()
            scope.cancel()
        }
    }

    @Test
    fun openingAFinishedPaneTellsTheNodeWhichFinishWasRead() {
        val node = StubNode().apply { start() }
        try {
            connected(node, MemoryPrefs()) { state ->
                node.push(finished("7"))
                until("the herd") { state.store.herd.value.panes.isNotEmpty() }

                state.openPane(PANE)

                until("the read to reach the node") { node.reads().isNotEmpty() }
                assertEquals(
                    """{"t":"done.read","pane":"$PANE","at":"7"}""",
                    node.reads().single(),
                )
            }
        } finally {
            node.stop()
        }
    }

    // A pane read while the socket was down, or on a build before the node could be told, is still
    // read — and the node has to hear it, or the phone keeps saying so.
    @Test
    fun aFinishThisDeviceHadAlreadyReadIsToldToTheNodeOnceItIsConnected() {
        val prefs = MemoryPrefs()
        val earlier = AppState(CoroutineScope(SupervisorJob()), KamprStore(), prefs, null, NoPush())
        earlier.store.accept(Wire.decode(finished("7")) ?: error("undecodable"))
        earlier.openPane(PANE)

        val node = StubNode().apply { start() }
        try {
            connected(node, prefs) { state ->
                node.push(finished("7"))
                until("the read to reach the node") { node.reads().isNotEmpty() }
                node.push(finished("7"))
                delay(300)
                assertEquals(1, node.reads().size, "told once is enough: ${node.reads()}")
            }
        } finally {
            node.stop()
        }
    }

    // A frame sent as the socket went is a frame the node may never have read. Told is per
    // connection, so the next one says it again.
    @Test
    fun aReadIsToldAgainOnTheNextConnection() {
        val node = StubNode().apply { start() }
        try {
            connected(node, MemoryPrefs()) { state ->
                node.push(finished("7"))
                until("the herd") { state.store.herd.value.panes.isNotEmpty() }
                state.openPane(PANE)
                until("the first telling") { node.reads().size == 1 }

                node.stop()
                until("the drop", timeoutMs = 30_000) { state.store.status.value !is ConnectionStatus.Live }
                node.start()
                until("the reconnect", timeoutMs = 30_000) { state.store.status.value is ConnectionStatus.Live }
                node.push(finished("7"))

                until("the second telling") { node.reads().size == 2 }
            }
        } finally {
            node.stop()
        }
    }
}
