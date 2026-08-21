package dev.kampr.shared

import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import dev.kampr.shared.wire.ClientMsg
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

class OutboxTest {
    // The audit killed the node mid-typing: 136 of 200 keystrokes vanished with no signal, and the
    // 64 that fitted in the outbox were replayed into the live shell twenty seconds later.
    @Test
    fun keystrokesTypedWhileTheSocketIsDownAreDroppedAndSaidSoRatherThanReplayed() {
        val port = freePort()
        val node = StubNode(port)
        node.start()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        try {
            runBlocking {
                store.pane(PANE)
                connection.connect(Endpoint("http://127.0.0.1:$port"))
                until("hello") { store.hello.value != null }
                connection.watch(PANE)
                connection.send(ClientMsg.InputText(PANE, "MARK"))
                until("the first keystroke") { node.received.any { it.contains("MARK") } }

                node.stop()
                until("the socket to drop") { store.status.value !is dev.kampr.shared.model.ConnectionStatus.Live }
                repeat(200) { connection.send(ClientMsg.InputText(PANE, "x")) }
                delay(300)

                node.start()
                until("the reconnect", timeoutMs = 30_000) {
                    store.status.value is dev.kampr.shared.model.ConnectionStatus.Live
                }
                delay(1_500)

                val typed = node.received.count { it.contains("\"x\"") }
                assertEquals(0, typed, "keystrokes typed while offline were replayed into a live shell")
                assertEquals(200, store.pane(PANE).undelivered, "the drop was never signalled")

                // A watch is a standing intent and does survive: the pane has to come back.
                until("the pane to be re-watched") {
                    node.received.count { it.contains("\"watch\"") } >= 2
                }
                connection.send(ClientMsg.InputText(PANE, "AFTER"))
                until("typing to work again") { node.received.any { it.contains("AFTER") } }
                assertEquals(0, store.pane(PANE).undelivered, "the warning has to clear once input lands")
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }

    @Test
    fun inputSentBeforeThereIsEverASocketIsDroppedRatherThanQueued() {
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        store.pane(PANE)
        connection.send(ClientMsg.InputText(PANE, "ghost"))
        assertTrue(store.pane(PANE).undelivered > 0)
        scope.cancel()
    }
}
