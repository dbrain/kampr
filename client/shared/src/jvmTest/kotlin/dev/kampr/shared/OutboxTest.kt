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
import java.io.InputStream
import java.io.OutputStream
import java.net.ServerSocket
import java.net.Socket
import java.security.MessageDigest
import java.util.Base64
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private const val PANE = "01JNODE/w1:p1"

// A ktor client talking to a real socket, because the defect is what survives the socket dying:
// a fake transport that never dies agrees with the code about a case neither of them has.
private class StubNode(val port: Int) {
    val received = CopyOnWriteArrayList<String>()
    private var server: ServerSocket? = null
    private val clients = CopyOnWriteArrayList<Socket>()

    fun start() {
        val socket = ServerSocket(port)
        server = socket
        thread(isDaemon = true) {
            while (!socket.isClosed) {
                val client = runCatching { socket.accept() }.getOrNull() ?: return@thread
                clients += client
                thread(isDaemon = true) { serve(client) }
            }
        }
    }

    fun stop() {
        server?.close()
        server = null
        for (client in clients) runCatching { client.close() }
        clients.clear()
    }

    private fun serve(client: Socket) {
        val input = client.getInputStream()
        val output = client.getOutputStream()
        val key = handshake(input) ?: return
        val accept = Base64.getEncoder().encodeToString(
            MessageDigest.getInstance("SHA-1")
                .digest((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").toByteArray()),
        )
        output.write(
            ("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n" +
                "Sec-WebSocket-Accept: $accept\r\n\r\n").toByteArray()
        )
        output.flush()
        send(output, HELLO)
        while (!client.isClosed) {
            val frame = readFrame(input) ?: return
            if (frame.first == 8) return
            if (frame.first == 1) received += frame.second.decodeToString()
        }
    }

    private fun handshake(input: InputStream): String? {
        val header = StringBuilder()
        while (!header.endsWith("\r\n\r\n")) {
            val byte = input.read()
            if (byte < 0) return null
            header.append(byte.toChar())
        }
        return header.lines()
            .firstOrNull { it.startsWith("Sec-WebSocket-Key:", ignoreCase = true) }
            ?.substringAfter(':')?.trim()
    }

    private fun send(output: OutputStream, text: String) {
        val body = text.toByteArray()
        val head = when {
            body.size < 126 -> byteArrayOf(0x81.toByte(), body.size.toByte())
            else -> byteArrayOf(0x81.toByte(), 126, (body.size shr 8).toByte(), body.size.toByte())
        }
        synchronized(output) {
            output.write(head)
            output.write(body)
            output.flush()
        }
    }

    private fun readFrame(input: InputStream): Pair<Int, ByteArray>? {
        val first = input.read()
        if (first < 0) return null
        val second = input.read()
        if (second < 0) return null
        val masked = second and 0x80 != 0
        var length = second and 0x7f
        if (length == 126) length = (input.read() shl 8) or input.read()
        else if (length == 127) {
            length = 0
            repeat(8) { length = (length shl 8) or input.read() }
        }
        val mask = if (masked) ByteArray(4).also { input.readNBytes(it, 0, 4) } else null
        val body = ByteArray(length)
        var read = 0
        while (read < length) {
            val n = input.read(body, read, length - read)
            if (n < 0) return null
            read += n
        }
        mask?.let { for (i in body.indices) body[i] = (body[i].toInt() xor it[i % 4].toInt()).toByte() }
        return (first and 0x0f) to body
    }
}

// A real `hello` off a real node, so the stub is answering with what the client actually meets.
private const val HELLO =
    """{"build":"0.1.0","caps":{"conversation":true,"manage":false,"mesh":false,"push":true,""" +
        """"scrollback":true},"node_id":"01JNODE","node_name":"stub","protocol":1,"role":"full",""" +
        """"security":{"encrypted":true,"installable":true,"origin":"http://127.0.0.1",""" +
        """"passkeys":false,"push":true,"tier":0,"unencrypted_banner":false,"unlocks":[]},""" +
        """"t":"hello"}"""

private fun freePort(): Int = ServerSocket(0).use { it.localPort }

private suspend fun until(what: String, timeoutMs: Long = 10_000, check: () -> Boolean) {
    val deadline = System.currentTimeMillis() + timeoutMs
    while (System.currentTimeMillis() < deadline) {
        if (check()) return
        delay(25)
    }
    throw AssertionError("timed out waiting for $what")
}

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
