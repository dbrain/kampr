package dev.kampr.shared

import dev.kampr.shared.net.AttachmentApi
import dev.kampr.shared.net.AttachmentBytes
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.createHttpClient
import kotlinx.coroutines.runBlocking
import java.net.ServerSocket
import java.net.Socket
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertContains
import kotlin.test.assertEquals
import kotlin.test.assertTrue

private val PIXELS = byteArrayOf(0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 7, 7, 7)

// A real socket, because what an attachment fetch can get wrong is entirely in the request line
// and the headers: a route the node never registered, and a bearer that never left the device.
private class NodeWithOneAttachment {
    var port: Int = 0
        private set

    val requests = CopyOnWriteArrayList<String>()
    val bearers = CopyOnWriteArrayList<String>()
    private var server: ServerSocket? = null

    fun start() {
        val socket = ServerSocket(0)
        port = socket.localPort
        server = socket
        thread(isDaemon = true) {
            while (!socket.isClosed) {
                val client = runCatching { socket.accept() }.getOrNull() ?: return@thread
                thread(isDaemon = true) { answer(client) }
            }
        }
    }

    fun stop() {
        server?.close()
        server = null
    }

    private fun answer(client: Socket): Unit = client.use {
        val header = StringBuilder()
        while (!header.endsWith("\r\n\r\n")) {
            val byte = it.getInputStream().read()
            if (byte < 0) return
            header.append(byte.toChar())
        }
        val lines = header.lines()
        val path = lines.firstOrNull()?.split(' ')?.getOrNull(1).orEmpty()
        requests += path
        lines.firstOrNull { line -> line.startsWith("Authorization:", ignoreCase = true) }
            ?.substringAfter(':')?.trim()?.let(bearers::add)
        val out = it.getOutputStream()
        if (path.endsWith("/att-7f3")) {
            out.write(
                ("HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: ${PIXELS.size}\r\n" +
                    "Connection: close\r\n\r\n").toByteArray()
            )
            out.write(PIXELS)
        } else {
            out.write("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".toByteArray())
        }
        out.flush()
    }
}

class AttachmentRouteTest {
    private fun <T> withNode(block: (NodeWithOneAttachment, AttachmentApi) -> T): T {
        val node = NodeWithOneAttachment()
        node.start()
        val client = createHttpClient()
        return try {
            block(node, AttachmentApi(client, Endpoint("http://127.0.0.1:${node.port}", "kmp_live")))
        } finally {
            client.close()
            node.stop()
        }
    }

    // The pane id carries a slash of its own and it stays a slash: `/api/attachment/{pane}/{id}`
    // spells the pane the way every frame on the socket spells it.
    @Test
    fun theFetchAsksTheAttachmentRouteForThatPaneAndCarriesTheDeviceToken() = withNode { node, api ->
        val got = runBlocking { api.fetch("01JNODE/w3:p2", "att-7f3") }
        assertEquals("/api/attachment/01JNODE/w3:p2/att-7f3", node.requests.single())
        assertEquals("Bearer kmp_live", node.bearers.single())
        assertTrue(got is AttachmentBytes.Ok)
        assertEquals(PIXELS.toList(), got.bytes.toList())
        assertEquals("image/png", got.mime)
    }

    // The node forgetting an id is the ordinary case — a transcript outlives the bytes it mentions
    // — and it has to arrive as a sentence rather than as an empty picture.
    @Test
    fun anIdTheNodeCannotResolveComesBackAsAReasonRatherThanAsNoBytes() = withNode { _, api ->
        val got = runBlocking { api.fetch("01JNODE/w3:p2", "att-gone") }
        assertTrue(got is AttachmentBytes.Failed, "a 404 was reported as bytes")
        assertContains(got.reason, "no longer has")
    }

    @Test
    fun aNodeThatIsNotThereIsAReasonTooRatherThanAnExceptionOnAPress() {
        val dead = portWithNothingOnIt()
        val client = createHttpClient()
        val got = try {
            runBlocking { AttachmentApi(client, Endpoint("http://127.0.0.1:$dead", "kmp_live")).fetch("p", "a") }
        } finally {
            client.close()
        }
        assertTrue(got is AttachmentBytes.Failed)
        assertContains(got.reason, "Could not reach the node")
    }
}
