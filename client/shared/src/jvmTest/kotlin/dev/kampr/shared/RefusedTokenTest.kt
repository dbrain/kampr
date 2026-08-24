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
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertTrue
import kotlin.test.fail

// A node that is up and has never heard of this device: every route, the socket upgrade included,
// is a 401. What a revoked device, a replaced database and an expired token all look like.
private class RefusingNode {
    var port: Int = 0
        private set

    val paths = CopyOnWriteArrayList<String>()
    private var server: ServerSocket? = null

    fun start() {
        val socket = ServerSocket(port)
        port = socket.localPort
        server = socket
        thread(isDaemon = true) {
            while (!socket.isClosed) {
                val client = runCatching { socket.accept() }.getOrNull() ?: return@thread
                thread(isDaemon = true) { refuse(client) }
            }
        }
    }

    fun stop() {
        server?.close()
        server = null
    }

    private fun refuse(client: Socket) = client.use {
        val header = StringBuilder()
        while (!header.endsWith("\r\n\r\n")) {
            val byte = it.getInputStream().read()
            if (byte < 0) return
            header.append(byte.toChar())
        }
        header.lines().firstOrNull()?.split(' ')?.getOrNull(1)?.let(paths::add)
        val body = "this device is not authorised".toByteArray()
        val out = it.getOutputStream()
        out.write(
            "HTTP/1.1 401 Unauthorized\r\nContent-Length: ${body.size}\r\nConnection: close\r\n\r\n".toByteArray()
        )
        out.write(body)
        out.flush()
    }
}

// A browser is never shown the status of a failed WebSocket handshake, so on the socket alone a
// node that has forgotten this device and a node that is switched off are the same event. That is
// what left a replaced database showing "reconnecting in 12s" over a cached herd, for ever, with
// nothing anywhere saying the token had been refused.
class RefusedTokenTest {
    @Test
    fun aTokenTheNodeRefusesIsDiagnosedRatherThanRetriedInSilence() {
        val node = RefusingNode()
        node.start()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        try {
            runBlocking {
                KamprConnection(scope, store).connect(Endpoint("http://127.0.0.1:${node.port}", "kmp_dead"))
                until("the refusal to be named") { store.status.value is ConnectionStatus.Refused }
                assertTrue(
                    node.paths.any { it != "/ws" },
                    "the refusal was guessed, not established: nothing but the socket was asked, and " +
                        "the socket cannot tell a 401 from an unplugged cable. Asked: ${node.paths}",
                )
            }
        } finally {
            scope.cancel()
            node.stop()
        }
    }

    // The guard that stops the fix above from being "any socket that will not open is a refusal".
    // A node that is simply down must still read as offline, because it comes back on its own and
    // telling the operator to pair again would be a lie that costs them their enrolment.
    @Test
    fun aNodeThatIsMerelyDownIsStillOfflineAndNeverARefusal() {
        val dead = portWithNothingOnIt()
        val scope = CoroutineScope(SupervisorJob())
        val store = KamprStore()
        try {
            runBlocking {
                KamprConnection(scope, store).connect(Endpoint("http://127.0.0.1:$dead", "kmp_valid"))
                until("an offline verdict") { store.status.value is ConnectionStatus.Offline }
                repeat(20) {
                    delay(100)
                    if (store.status.value is ConnectionStatus.Refused) {
                        fail("an unreachable node was reported as a refused device")
                    }
                }
            }
        } finally {
            scope.cancel()
        }
    }
}
