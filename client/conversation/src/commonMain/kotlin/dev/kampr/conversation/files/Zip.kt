package dev.kampr.conversation.files

class ZipEntry(val path: String, val size: Long, val packed: Long, val isDir: Boolean, val encrypted: Boolean, val modified: String?)

sealed interface ZipListing {
    class Ok(val entries: List<ZipEntry>, val total: Long) : ZipListing
    class Failed(val why: String) : ZipListing
}

const val ZIP_ENTRY_CAP = 50_000
private const val DIRECTORY_CAP = 8L * 1024 * 1024

private const val LOCAL = 0x04034b50L
private const val CENTRAL = 0x02014b50L
private const val END = 0x06054b50L
private const val END64 = 0x06064b50L
private const val LOCATOR64 = 0x07064b50L
private const val END_BYTES = 22
private const val MAX_COMMENT = 0xFFFF
private const val CENTRAL_BYTES = 46
private const val ZIP64_EXTRA = 1
private const val SATURATED = 0xFFFFFFFFL

private val CP437_HIGH =
    "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐" +
        "└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■ "

fun looksLikeZip(head: ByteArray): Boolean = head.size >= 4 && (head.u32(0) == LOCAL || head.u32(0) == END)

private class Damaged(val why: String) : Exception(why)

private fun ByteArray.u16(at: Int): Int {
    if (at < 0 || at + 2 > size) throw Damaged("it ends partway through a record")
    return (this[at].toInt() and 0xFF) or ((this[at + 1].toInt() and 0xFF) shl 8)
}

private fun ByteArray.u32(at: Int): Long = u16(at).toLong() or (u16(at + 2).toLong() shl 16)

private fun ByteArray.u64(at: Int): Long = u32(at) or (u32(at + 4) shl 32)

suspend fun listZip(size: Long, read: suspend (start: Long, endInclusive: Long) -> ByteArray?): ZipListing = try {
    parse(size, read)
} catch (damaged: Damaged) {
    ZipListing.Failed(damaged.why)
}

private suspend fun readFully(read: suspend (Long, Long) -> ByteArray?, start: Long, length: Long): ByteArray {
    if (length == 0L) return ByteArray(0)
    val out = ByteArray(length.toInt())
    var have = 0
    while (have < length) {
        val got = read(start + have, start + length - 1) ?: throw Damaged("its directory could not be read")
        if (got.isEmpty()) throw Damaged("it ends before its directory does")
        got.copyInto(out, have, 0, minOf(got.size, out.size - have))
        have += got.size
    }
    return out
}

private suspend fun parse(size: Long, read: suspend (Long, Long) -> ByteArray?): ZipListing.Ok {
    if (size < END_BYTES) throw Damaged("it is too short to hold an archive's end")
    val tailStart = maxOf(0L, size - END_BYTES - MAX_COMMENT)
    val tail = readFully(read, tailStart, size - tailStart)
    // The end record is followed only by a comment of up to 64 KiB that may itself hold the
    // signature, so the candidate whose comment reaches exactly to the end is preferred.
    val end = (tail.size - END_BYTES downTo 0).firstOrNull { at ->
        tail.u32(at) == END && at + END_BYTES + tail.u16(at + 20) == tail.size
    } ?: (tail.size - END_BYTES downTo 0).firstOrNull { tail.u32(it) == END }
        ?: throw Damaged("it has no end of central directory")

    var total = tail.u16(end + 10).toLong()
    var dirSize = tail.u32(end + 12)
    var dirStart = tail.u32(end + 16)
    var dirEnd = tailStart + end
    val locator = end - 20
    if (locator >= 0 && tail.u32(locator) == LOCATOR64) {
        val at = tail.u64(locator + 8)
        if (at < 0 || at > tailStart + locator - 56) throw Damaged("its ZIP64 end record lies outside it")
        val record = if (at >= tailStart) tail.copyOfRange((at - tailStart).toInt(), (at - tailStart).toInt() + 56) else readFully(read, at, 56)
        if (record.u32(0) != END64) throw Damaged("its ZIP64 end record is missing")
        total = record.u64(32)
        dirSize = record.u64(40)
        dirStart = record.u64(48)
        dirEnd = at
    }
    if (total < 0 || dirSize < 0 || dirStart < 0 || dirSize > dirEnd - dirStart) throw Damaged("its directory lies outside it")
    val wanted = minOf(total, ZIP_ENTRY_CAP.toLong())
    if (dirSize < wanted * CENTRAL_BYTES) throw Damaged("its directory is too short for the $total entries it claims")

    val dir = when {
        dirSize > DIRECTORY_CAP -> readFully(read, dirStart, DIRECTORY_CAP)
        dirStart >= tailStart -> tail.copyOfRange((dirStart - tailStart).toInt(), (dirStart - tailStart + dirSize).toInt())
        else -> readFully(read, dirStart, dirSize)
    }
    val whole = dirSize <= DIRECTORY_CAP
    val entries = ArrayList<ZipEntry>(wanted.toInt())
    var at = 0
    while (entries.size < wanted) {
        if (!whole && at + CENTRAL_BYTES > dir.size) break
        val entry = entryAt(dir, at, whole) ?: break
        entries += entry.first
        at = entry.second
    }
    if (entries.isEmpty() && total > 0) throw Damaged("its directory could not be read")
    return ZipListing.Ok(entries, total)
}

// A directory read only to its cap may cut an entry in half; that entry is where the listing
// stops, not damage.
private fun entryAt(dir: ByteArray, at: Int, whole: Boolean): Pair<ZipEntry, Int>? {
    if (dir.u32(at) != CENTRAL) throw Damaged("its directory is garbled")
    val nameLength = dir.u16(at + 28)
    val extraLength = dir.u16(at + 30)
    val commentLength = dir.u16(at + 32)
    val next = at + CENTRAL_BYTES + nameLength + extraLength + commentLength
    if (next > dir.size) {
        if (whole) throw Damaged("an entry runs past the end of its directory")
        return null
    }
    val flags = dir.u16(at + 8)
    val nameBytes = dir.copyOfRange(at + CENTRAL_BYTES, at + CENTRAL_BYTES + nameLength)
    val path = if (flags and 0x800 != 0) nameBytes.decodeToString() else cp437(nameBytes)
    var size = dir.u32(at + 24)
    var packed = dir.u32(at + 20)
    var extra = at + CENTRAL_BYTES + nameLength
    val extraEnd = extra + extraLength
    while (extra + 4 <= extraEnd) {
        val id = dir.u16(extra)
        val length = dir.u16(extra + 2)
        if (extra + 4 + length > extraEnd) break
        if (id == ZIP64_EXTRA) {
            var field = extra + 4
            if (size == SATURATED && field + 8 <= extra + 4 + length) {
                size = dir.u64(field)
                field += 8
            }
            if (packed == SATURATED && field + 8 <= extra + 4 + length) packed = dir.u64(field)
        }
        extra += 4 + length
    }
    val entry = ZipEntry(path, size, packed, path.endsWith('/'), flags and 1 != 0, dosTime(dir.u16(at + 14), dir.u16(at + 12)))
    return entry to next
}

private fun cp437(bytes: ByteArray): String = buildString(bytes.size) {
    for (b in bytes) {
        val c = b.toInt() and 0xFF
        append(if (c < 0x80) c.toChar() else CP437_HIGH[c - 0x80])
    }
}

private fun dosTime(date: Int, time: Int): String? {
    val month = (date shr 5) and 0xF
    val day = date and 0x1F
    val hour = time shr 11
    val minute = (time shr 5) and 0x3F
    if (month !in 1..12 || day == 0 || hour > 23 || minute > 59) return null
    fun two(n: Int) = n.toString().padStart(2, '0')
    return "${1980 + (date shr 9)}-${two(month)}-${two(day)} ${two(hour)}:${two(minute)}"
}
