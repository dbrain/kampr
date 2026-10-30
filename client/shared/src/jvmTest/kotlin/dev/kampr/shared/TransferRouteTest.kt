package dev.kampr.shared

import dev.kampr.shared.net.AttachmentApi
import dev.kampr.shared.net.AttachmentSlice
import dev.kampr.shared.net.ContentRange
import dev.kampr.shared.net.DownloadSink
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.UploadResult
import dev.kampr.shared.net.UploadSource
import dev.kampr.shared.net.createHttpClient
import dev.kampr.shared.net.parseContentRange
import dev.kampr.shared.net.sliceOf
import dev.kampr.shared.net.uploadResultOf
import kotlinx.coroutines.runBlocking
import java.io.ByteArrayOutputStream
import java.io.InputStream
import java.net.ServerSocket
import java.net.Socket
import java.net.URLDecoder
import java.util.concurrent.CopyOnWriteArrayList
import kotlin.concurrent.thread
import kotlin.test.Test
import kotlin.test.assertContentEquals
import kotlin.test.assertEquals
import kotlin.test.assertIs
import kotlin.test.assertTrue

private val BIG = ByteArray(3 * 1024 * 1024 + 17) { (it * 31 % 251).toByte() }

// Ranges, a whole body large enough to arrive in many reads, and an upload — over a real socket,
// because what these get wrong is in the request line, the headers and how the body is framed.
private class FileNode {
    var port = 0
        private set
    val requests = CopyOnWriteArrayList<String>()
    val ranges = CopyOnWriteArrayList<String>()
    val uploaded = ByteArrayOutputStream()
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

    fun stop() = server?.close()

    private fun head(input: InputStream): List<String>? {
        val header = StringBuilder()
        while (!header.endsWith("\r\n\r\n")) {
            val byte = input.read()
            if (byte < 0) return null
            header.append(byte.toChar())
        }
        return header.lines()
    }

    private fun answer(client: Socket): Unit = client.use {
        val input = it.getInputStream()
        val lines = head(input) ?: return
        val (method, path) = lines.first().split(' ')
        requests += "$method $path"
        fun header(name: String) = lines.firstOrNull { line -> line.startsWith("$name:", ignoreCase = true) }?.substringAfter(':')?.trim()
        val out = it.getOutputStream()
        if (method == "POST") {
            val length = header("Content-Length")?.toLong() ?: -1
            val body = if (length >= 0) input.readNBytes(length.toInt()) else dechunk(input)
            uploaded.write(body)
            val json = """{"path":"/home/u/in/notes (1).txt","bytes":${body.size}}"""
            out.write("HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: ${json.length}\r\nConnection: close\r\n\r\n$json".toByteArray())
            return
        }
        val range = header("Range")
        if (range != null) {
            ranges += range
            val (a, b) = range.removePrefix("bytes=").split('-').map(String::toLong)
            val end = minOf(b, BIG.size - 1L)
            val slice = BIG.copyOfRange(a.toInt(), end.toInt() + 1)
            out.write("HTTP/1.1 206 Partial Content\r\nContent-Type: text/plain\r\nContent-Range: bytes $a-$end/${BIG.size}\r\nContent-Length: ${slice.size}\r\nConnection: close\r\n\r\n".toByteArray())
            out.write(slice)
        } else {
            out.write("HTTP/1.1 200 OK\r\nContent-Type: application/zip\r\nContent-Length: ${BIG.size}\r\nConnection: close\r\n\r\n".toByteArray())
            BIG.toList().chunked(64 * 1024).forEach { part ->
                out.write(part.toByteArray())
                out.flush()
            }
        }
        out.flush()
    }

    private fun dechunk(input: InputStream): ByteArray {
        val body = ByteArrayOutputStream()
        while (true) {
            val size = generateSequence { input.read().takeIf { b -> b >= 0 }?.toChar() }.takeWhile { c -> c != '\n' }.joinToString("").trim().toInt(16)
            if (size == 0) return body.toByteArray()
            body.write(input.readNBytes(size))
            input.read()
            input.read()
        }
    }
}

private class Collecting : DownloadSink {
    val bytes = ByteArrayOutputStream()
    var writes = 0
    var mime: String? = null
    override suspend fun write(chunk: ByteArray, length: Int) {
        writes++
        bytes.write(chunk, 0, length)
    }
    override suspend fun finish(): String = "Downloads/x.zip"
    override suspend fun abort() = Unit
}

private class Bytes(private val all: ByteArray) : UploadSource {
    private var at = 0
    var closed = false
    override val name = "notes.txt"
    override val size: Long = all.size.toLong()
    override suspend fun read(into: ByteArray): Int {
        if (at >= all.size) return -1
        val n = minOf(into.size, all.size - at)
        all.copyInto(into, 0, at, at + n)
        at += n
        return n
    }
    override suspend fun close() {
        closed = true
    }
}

class TransferRouteTest {
    private fun <T> withNode(block: suspend (FileNode, AttachmentApi) -> T): T {
        val node = FileNode()
        node.start()
        val client = createHttpClient()
        return try {
            runBlocking { block(node, AttachmentApi(client, Endpoint("http://127.0.0.1:${node.port}", "kmp_live"))) }
        } finally {
            client.close()
            node.stop()
        }
    }

    @Test
    fun aRangeAsksForInclusiveBytesAndHandsBackTheSliceAndTheWholeSize() = withNode { node, api ->
        val got = api.range("01JNODE/w3:p2", "att", 1_000, 1_999)
        assertEquals("bytes=1000-1999", node.ranges.single())
        assertIs<AttachmentSlice.Ok>(got)
        assertContentEquals(BIG.copyOfRange(1_000, 2_000), got.bytes)
        assertEquals(BIG.size.toLong(), got.total)
    }

    @Test
    fun aDownloadArrivesInChunksRatherThanAsOneBody() = withNode { _, api ->
        val sink = Collecting()
        val where = api.download("01JNODE/w3:p2", "zip") { mime ->
            sink.mime = mime
            sink
        }
        assertEquals("Downloads/x.zip", where)
        assertEquals("application/zip", sink.mime)
        assertContentEquals(BIG, sink.bytes.toByteArray())
        assertTrue(sink.writes > 1, "the body reached the sink in ${sink.writes} write")
    }

    @Test
    fun anUploadPostsTheBytesToTheFolderAndSaysWhereTheyLanded() = withNode { node, api ->
        val body = BIG.copyOfRange(0, 700_000)
        val source = Bytes(body)
        val seen = mutableListOf<Long>()
        val got = api.upload("01JNODE/_", "~/in", "notes.txt", source) { seen += it }
        val request = node.requests.single()
        assertTrue(request.startsWith("POST /api/upload/01JNODE/_?"), request)
        assertEquals("dir=~/in&name=notes.txt", URLDecoder.decode(request.substringAfter('?'), "UTF-8"))
        assertContentEquals(body, node.uploaded.toByteArray())
        assertEquals(UploadResult.Ok("/home/u/in/notes (1).txt", 700_000), got)
        assertEquals(700_000L, seen.last())
        assertTrue(seen.size > 1, "progress was reported $seen")
        assertTrue(source.closed)
    }

    @Test
    fun contentRangeHeadersParse() {
        val table = mapOf(
            "bytes 0-99/1000" to ContentRange(0, 99, 1000),
            "bytes 5-9/*" to ContentRange(5, 9, null),
            "bytes */1000" to ContentRange(null, null, 1000),
            "items 0-1/2" to null,
            "" to null,
        )
        for ((header, want) in table) assertEquals(want, parseContentRange(header), header)
    }

    @Test
    fun aNodeThatIgnoredTheRangeHasTheSliceCutOutOfTheWhole() {
        val whole = "0123456789".encodeToByteArray()
        val got = sliceOf(200, null, "text/plain", whole, 4, 6)
        assertIs<AttachmentSlice.Ok>(got)
        assertEquals("456", got.bytes.decodeToString())
        assertEquals(10L, got.total)
    }

    @Test
    fun aRangePastTheEndIsAnEmptySliceWithTheSizeAndADifferentStartIsRefused() {
        val past = sliceOf(416, "bytes */10", null, ByteArray(0), 10, 20)
        assertIs<AttachmentSlice.Ok>(past)
        assertEquals(0, past.bytes.size)
        assertEquals(10L, past.total)
        assertIs<AttachmentSlice.Failed>(sliceOf(206, "bytes 0-9/10", null, ByteArray(10), 5, 9))
        assertEquals(true, (sliceOf(404, null, null, ByteArray(0), 0, 9) as AttachmentSlice.Failed).missing)
    }

    @Test
    fun uploadRefusalsAreSentences() {
        assertEquals(UploadResult.Ok("/a/b", 3), uploadResultOf(201, """{"path":"/a/b","bytes":3}"""))
        for (status in listOf(400, 403, 404, 500)) assertIs<UploadResult.Failed>(uploadResultOf(status, ""), "$status")
        assertIs<UploadResult.Failed>(uploadResultOf(201, "not json"))
        assertEquals(UploadResult.Failed("Nothing arrived for a minute, so the node gave up on the upload."), uploadResultOf(408, ""))
    }
}
