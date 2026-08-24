package dev.kampr.shared

import kotlinx.coroutines.delay
import java.io.InputStream
import java.io.OutputStream
import java.net.ServerSocket
import java.net.Socket
import java.security.MessageDigest
import java.util.Base64
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread

// A ktor client talking to a real socket, because the defect is what survives the socket dying:
// a fake transport that never dies agrees with the code about a case neither of them has.
internal class StubNode(private val greeting: String = HELLO) {
    val received = CopyOnWriteArrayList<String>()
    private var server: ServerSocket? = null
    private val clients = CopyOnWriteArrayList<Socket>()

    // Bound before it is known, never chosen and then bound. Asking the kernel for a free port and
    // binding it in a second step leaves a window for anything else on the machine to take it, and
    // on a loaded runner something does. Zero on the first start, and the port it was given on
    // every restart after that, because a test that stops the node and brings it back has to bring
    // it back to the address its client is still dialling.
    var port: Int = 0
        private set

    fun start() {
        val socket = ServerSocket(port)
        port = socket.localPort
        server = socket
        thread(isDaemon = true) {
            while (!socket.isClosed) {
                val client = runCatching { socket.accept() }.getOrNull() ?: return@thread
                clients += client
                thread(isDaemon = true) { serve(client) }
            }
        }
    }

    // Pushes one server frame to every connected client. `hello` goes out on accept; anything
    // a test wants to arrive *later* goes through here.
    fun push(text: String) {
        for (client in clients) runCatching { send(client.getOutputStream(), text) }
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
        send(output, greeting)
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
internal const val HELLO =
    """{"build":"0.1.0","caps":{"conversation":true,"manage":false,"mesh":false,"push":true,""" +
        """"scrollback":true},"node_id":"01JNODE","node_name":"stub","protocol":1,"role":"full",""" +
        """"security":{"encrypted":true,"installable":true,"origin":"http://127.0.0.1",""" +
        """"passkeys":false,"push":true,"tier":0,"unencrypted_banner":false,"unlocks":[]},""" +
        """"t":"hello"}"""

// An address with nothing listening on it, for the one test that needs a node that is down. This
// is a guess by construction — the port is free when it is asked for and could be taken before it
// is dialled — but it is a fail-safe one: a stranger answering there makes the test fail rather
// than pass. Anything that wants a port to *listen* on binds it itself; see `StubNode.port`.
internal fun portWithNothingOnIt(): Int = ServerSocket(0).use { it.localPort }

internal suspend fun until(what: String, timeoutMs: Long = 10_000, check: () -> Boolean) {
    val deadline = System.currentTimeMillis() + timeoutMs
    while (System.currentTimeMillis() < deadline) {
        if (check()) return
        delay(25)
    }
    throw AssertionError("timed out waiting for $what")
}
