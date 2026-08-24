package dev.kampr.shared

import dev.kampr.shared.model.ConnectionStatus
import dev.kampr.shared.model.KamprStore
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.KamprConnection
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.runBlocking
import kotlin.test.Test
import kotlin.test.assertTrue

// The backoff was created once per connect and only ever asked for its next delay: `reset()` had
// exactly one caller in the whole client, a unit test. So a phone that drops and regains its
// network a few times over a day waits the full fifteen seconds before every attempt afterwards,
// however long and healthy the sessions in between were.
class ReconnectTest {
    @Test
    fun aSessionThatStayedUpEarnsTheShortRetryBackAgain() {
        val node = StubNode()
        node.start()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        val connection = KamprConnection(scope, store)
        try {
            runBlocking {
                connection.connect(Endpoint("http://127.0.0.1:${node.port}"))
                until("hello") { store.hello.value != null }

                node.stop()
                until("the retry to escalate", timeoutMs = 30_000) {
                    (store.status.value as? ConnectionStatus.Offline)?.retryInMs?.let { it > 500 } == true
                }

                node.start()
                until("the reconnect", timeoutMs = 30_000) { store.status.value is ConnectionStatus.Live }
                delay(6_000)

                node.stop()
                until("the second drop", timeoutMs = 30_000) { store.status.value is ConnectionStatus.Offline }
                val wait = (store.status.value as ConnectionStatus.Offline).retryInMs
                assertTrue(
                    wait <= 250,
                    "a session that stayed up for six seconds still left the next retry at $wait ms",
                )
            }
        } finally {
            connection.disconnect()
            scope.cancel()
            node.stop()
        }
    }
}
