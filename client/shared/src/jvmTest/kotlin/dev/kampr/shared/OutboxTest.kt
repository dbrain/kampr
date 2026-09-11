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
        val node = StubNode()
        node.start()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        try {
            runBlocking {
                store.pane(PANE)
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
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

    // **The two pane writes that are not keystrokes, and were filed as standing intents.**
    //
    // `typing` lists `input.text`, `input.b64`, `input.keys` and `answer`; everything else is
    // "still true after a reconnect" and keeps its place in the queue. `answer.submit` and `paste`
    // are not — both write to whatever is on the pane *now*.
    //
    // `answer.submit` is the dangerous one. The node turns it into right-arrow then Enter (#421)
    // and, unlike `answer`, takes **no read of the pane** first — `moves_to` exists precisely so
    // that "a dialog that has gone in the meantime answers nothing rather than pressing Enter into
    // whatever replaced it", and the commit path had no equivalent. So one queued across a dropped
    // socket lands twenty seconds later as Enter into a shell line the operator never submitted,
    // or into an agent's prompt box they were still writing in, with nothing on screen connecting
    // the two.
    //
    // Table-driven so the next pane write added to the wire has to be classified rather than
    // defaulting into the queue.
    @Test
    fun paneWritesQueuedWhileTheSocketIsDownAreDroppedRatherThanReplayed() {
        for ((wire, msg) in listOf(
            "answer.submit" to ClientMsg.AnswerSubmit(PANE),
            "paste" to ClientMsg.Paste(PANE, "aGk="),
        )) {
            val node = StubNode()
            node.start()
            val scope = CoroutineScope(SupervisorJob())
            val store = KamprStore()
            val connection = KamprConnection(scope, store)
            try {
                runBlocking {
                    store.pane(PANE)
                    connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                    until("hello") { store.hello.value != null }
                    connection.watch(PANE)

                    node.stop()
                    until("the socket to drop") {
                        store.status.value !is dev.kampr.shared.model.ConnectionStatus.Live
                    }
                    connection.send(msg)
                    delay(300)

                    node.start()
                    until("the reconnect", timeoutMs = 30_000) {
                        store.status.value is dev.kampr.shared.model.ConnectionStatus.Live
                    }
                    delay(1_500)

                    val replayed = node.received.count { it.contains("\"$wire\"") }
                    assertEquals(
                        0,
                        replayed,
                        "a $wire queued while the socket was down was replayed into the pane",
                    )
                }
            } finally {
                connection.disconnect()
                scope.cancel()
                node.stop()
            }
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
