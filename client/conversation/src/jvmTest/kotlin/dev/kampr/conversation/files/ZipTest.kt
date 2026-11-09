package dev.kampr.conversation.files

import kotlinx.coroutines.runBlocking
import java.io.ByteArrayOutputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.nio.charset.Charset
import java.time.LocalDateTime
import java.util.zip.CRC32
import java.util.zip.ZipOutputStream
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue
import kotlin.test.fail
import java.util.zip.ZipEntry as JavaEntry

internal fun zipOf(charset: Charset = Charsets.UTF_8, comment: String? = null, build: ZipOutputStream.() -> Unit): ByteArray {
    val out = ByteArrayOutputStream()
    ZipOutputStream(out, charset).use { zip ->
        comment?.let(zip::setComment)
        zip.build()
    }
    return out.toByteArray()
}

internal fun ZipOutputStream.file(name: String, body: ByteArray, stored: Boolean = false, at: LocalDateTime? = null) {
    val entry = JavaEntry(name)
    at?.let { entry.timeLocal = it }
    if (stored) {
        entry.method = JavaEntry.STORED
        entry.size = body.size.toLong()
        entry.compressedSize = body.size.toLong()
        entry.crc = CRC32().apply { update(body) }.value
    }
    putNextEntry(entry)
    write(body)
    closeEntry()
}

internal fun ZipOutputStream.folder(name: String) {
    putNextEntry(JavaEntry(name))
    closeEntry()
}

private fun listed(bytes: ByteArray, reads: MutableList<LongRange>? = null): ZipListing = runBlocking {
    listZip(bytes.size.toLong()) { start, end ->
        reads?.add(start..end)
        if (start < 0 || end >= bytes.size || start > end) null else bytes.copyOfRange(start.toInt(), end.toInt() + 1)
    }
}

private fun ok(bytes: ByteArray): ZipListing.Ok = when (val got = listed(bytes)) {
    is ZipListing.Ok -> got
    is ZipListing.Failed -> fail("listing failed: ${got.why}")
}

private fun le(size: Int, fill: ByteBuffer.() -> Unit): ByteArray = ByteBuffer.allocate(size).order(ByteOrder.LITTLE_ENDIAN).apply(fill).array()

private fun indexOf(bytes: ByteArray, magic: ByteArray, from: Int = 0): Int = (from..bytes.size - magic.size).first { at -> magic.indices.all { bytes[at + it] == magic[it] } }

private val CENTRAL = byteArrayOf(0x50, 0x4b, 0x01, 0x02)

class ZipTest {
    @Test
    fun namesSizesFoldersAndTimesAreReadFromTheCentralDirectory() {
        val body = "the fourth hop drops it\n".repeat(200).encodeToByteArray()
        val noon = LocalDateTime.of(2024, 3, 5, 14, 22, 18)
        val bytes = zipOf {
            folder("src/")
            file("src/notes.txt", body, at = noon)
            file("raw.bin", byteArrayOf(1, 2, 3), stored = true, at = noon)
        }
        val got = ok(bytes)
        assertEquals(3L, got.total)
        assertEquals(listOf("src/", "src/notes.txt", "raw.bin"), got.entries.map { it.path })
        assertEquals(listOf(true, false, false), got.entries.map { it.isDir })
        val notes = got.entries[1]
        assertEquals(body.size.toLong(), notes.size)
        assertTrue(notes.packed in 1 until body.size, "a deflated entry's packed size was ${notes.packed}")
        assertEquals("2024-03-05 14:22", notes.modified)
        assertEquals(3L, got.entries[2].size)
        assertEquals(3L, got.entries[2].packed)
        assertTrue(got.entries.none { it.encrypted })
    }

    @Test
    fun theZipMagicIsALocalHeaderOrAnEmptyArchivesEnd() {
        assertTrue(looksLikeZip(zipOf { file("a", byteArrayOf(1)) }))
        assertTrue(looksLikeZip(zipOf { }))
        assertTrue(!looksLikeZip("PK is just two letters".encodeToByteArray()))
        assertTrue(!looksLikeZip(byteArrayOf(0x50, 0x4b)))
    }

    @Test
    fun anEmptyArchiveListsNothing() {
        val bytes = zipOf { }
        assertEquals(22, bytes.size)
        val got = ok(bytes)
        assertEquals(0L, got.total)
        assertTrue(got.entries.isEmpty())
    }

    @Test
    fun aNameWithTheUtf8FlagIsUtf8() {
        val got = ok(zipOf { file("naïve/日本.txt", byteArrayOf(1)) })
        assertEquals("naïve/日本.txt", got.entries.single().path)
    }

    @Test
    fun aNameWithoutTheUtf8FlagIsCodePage437() {
        val high = String(ByteArray(128) { (0x80 + it).toByte() }, charset("IBM437"))
        val got = ok(zipOf(charset("IBM437")) { file("dos/$high", byteArrayOf(1)) })
        assertEquals("dos/$high", got.entries.single().path)
    }

    @Test
    fun anArchiveCommentDoesNotHideTheEnd() {
        val got = ok(zipOf(comment = "PK\u0005\u0006 opens this comment, which runs on long enough to pass for an end record") { file("a.txt", byteArrayOf(1)) })
        assertEquals(listOf("a.txt"), got.entries.map { it.path })
    }

    @Test
    fun anEncryptedEntrySaysSo() {
        val bytes = zipOf { file("secret.txt", byteArrayOf(1, 2)) }
        val central = indexOf(bytes, CENTRAL)
        bytes[central + 8] = (bytes[central + 8].toInt() or 1).toByte()
        assertTrue(ok(bytes).entries.single().encrypted)
    }

    // Past 65,535 entries the 16-bit counts overflow and Java writes the ZIP64 end record and its
    // locator; the listing stops at its cap and says how many there are.
    @Test
    fun aZip64ArchiveIsListedUpToTheCap() {
        val bytes = zipOf { repeat(65_536) { file("f$it", ByteArray(0), stored = true) } }
        indexOf(bytes, byteArrayOf(0x50, 0x4b, 0x06, 0x07), bytes.size - 200)
        val got = ok(bytes)
        assertEquals(65_536L, got.total)
        assertEquals(ZIP_ENTRY_CAP, got.entries.size)
        assertEquals("f0", got.entries.first().path)
        assertEquals("f${ZIP_ENTRY_CAP - 1}", got.entries.last().path)
    }

    // A file past 4 GB carries its sizes in the ZIP64 extra field: built by hand, since writing one
    // with java.util.zip would take 4 GB.
    @Test
    fun aZip64ExtraFieldCarriesSizesPastFourGigabytes() {
        val name = "huge.iso".encodeToByteArray()
        val extra = le(20) {
            putShort(1)
            putShort(16)
            putLong(5_000_000_000L)
            putLong(4_900_000_000L)
        }
        val central = le(46 + name.size + extra.size) {
            putInt(0x02014b50)
            putShort(45)
            putShort(45)
            putShort(0)
            putShort(8)
            putShort(0)
            putShort(0)
            putInt(0)
            putInt(-1)
            putInt(-1)
            putShort(name.size.toShort())
            putShort(extra.size.toShort())
            putShort(0)
            putShort(0)
            putShort(0)
            putInt(0)
            putInt(0)
            put(name)
            put(extra)
        }
        val head = byteArrayOf(0x50, 0x4b, 0x03, 0x04) + ByteArray(26)
        val end = le(22) {
            putInt(0x06054b50)
            putShort(0)
            putShort(0)
            putShort(1)
            putShort(1)
            putInt(central.size)
            putInt(head.size)
            putShort(0)
        }
        val entry = ok(head + central + end).entries.single()
        assertEquals("huge.iso", entry.path)
        assertEquals(5_000_000_000L, entry.size)
        assertEquals(4_900_000_000L, entry.packed)
    }

    @Test
    fun onlyTheTailAndTheDirectoryAreRead() {
        val bytes = zipOf {
            file("big.bin", ByteArray(3_000_000) { (it * 31).toByte() }, stored = true)
            file("small.txt", byteArrayOf(1))
        }
        val reads = mutableListOf<LongRange>()
        assertEquals(listOf("big.bin", "small.txt"), (listed(bytes, reads) as ZipListing.Ok).entries.map { it.path })
        val read = reads.sumOf { it.last - it.first + 1 }
        assertTrue(read < 70_000, "read $read of ${bytes.size} bytes to list two entries")
    }

    @Test
    fun damageIsAReasonNeverAThrow() {
        val good = zipOf {
            file("a.txt", "alpha".encodeToByteArray())
            file("b.txt", "beta".encodeToByteArray())
        }
        val central = indexOf(good, CENTRAL)
        val table = mapOf(
            "cut short" to good.copyOf(good.size - 10),
            "directory missing" to good.copyOf(central) + good.copyOfRange(good.size - 22, good.size),
            "directory garbled" to good.copyOf().also { it.fill(0x41, central, central + 12) },
            "nothing but the magic" to byteArrayOf(0x50, 0x4b, 0x03, 0x04),
            "name runs off the end" to good.copyOf().also {
                it[central + 28] = 0xff.toByte()
                it[central + 29] = 0x7f
            },
            "count larger than the directory" to good.copyOf().also {
                it[good.size - 12] = 9
                it[good.size - 14] = 9
            },
        )
        for ((label, bytes) in table) {
            val got = runCatching { listed(bytes) }.getOrElse { fail("$label threw $it") }
            assertTrue(got is ZipListing.Failed, "$label listed ${(got as? ZipListing.Ok)?.entries?.map { it.path }}")
        }
        val refused = runBlocking { listZip(good.size.toLong()) { _, _ -> null } }
        assertTrue(refused is ZipListing.Failed, "a reader that refused still listed")
    }
}
