package dev.kampr.shared.net

import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

sealed interface AttachmentSlice {
    class Ok(val bytes: ByteArray, val mime: String?, val total: Long?) : AttachmentSlice
    data class Failed(val reason: String, val missing: Boolean = false) : AttachmentSlice
}

interface DownloadSink {
    suspend fun write(chunk: ByteArray, length: Int)

    // Where it landed, in words an operator can go and look at, or null if the device refused it.
    suspend fun finish(): String?

    suspend fun abort()
}

interface UploadSource {
    val name: String
    val size: Long?

    // -1 at the end, like every stream it stands in for.
    suspend fun read(into: ByteArray): Int

    suspend fun close()
}

// A browser cannot stream a request body through ktor's fetch engine — it reads the whole body
// into memory first — but `XMLHttpRequest.send(file)` streams straight off the disk and reports
// progress. A source that can send itself does.
interface DirectUpload {
    suspend fun post(url: String, bearer: String?, progress: (Long) -> Unit): Answer?

    class Answer(val status: Int, val body: String)
}

sealed interface UploadResult {
    data class Ok(val path: String, val bytes: Long) : UploadResult
    data class Failed(val reason: String) : UploadResult
}

data class ContentRange(val start: Long?, val endInclusive: Long?, val total: Long?)

fun rangeHeader(start: Long, endInclusive: Long): String = "bytes=$start-$endInclusive"

private val CONTENT_RANGE = Regex("""^bytes\s+(?:(\d+)-(\d+)|\*)/(\d+|\*)$""")

fun parseContentRange(header: String?): ContentRange? {
    val match = header?.trim()?.let(CONTENT_RANGE::find) ?: return null
    val (first, last, total) = match.destructured
    return ContentRange(first.toLongOrNull(), last.toLongOrNull(), total.toLongOrNull())
}

// An answer that ignored the Range header — an older node, or an id it does not range — is the
// whole thing, so the slice asked for is cut out of it here rather than handed back as though it
// started where the caller asked.
fun sliceOfWhole(bytes: ByteArray, mime: String?, start: Long, endInclusive: Long): AttachmentSlice.Ok {
    val from = start.coerceIn(0, bytes.size.toLong()).toInt()
    val to = (endInclusive + 1).coerceIn(from.toLong(), bytes.size.toLong()).toInt()
    return AttachmentSlice.Ok(bytes.copyOfRange(from, to), mime, bytes.size.toLong())
}

fun sliceOf(status: Int, contentRange: String?, mime: String?, body: ByteArray, start: Long, endInclusive: Long): AttachmentSlice = when (status) {
    200 -> sliceOfWhole(body, mime, start, endInclusive)

    206 -> {
        val range = parseContentRange(contentRange)
        if (range?.start != start) {
            AttachmentSlice.Failed("The node answered a different part of the file than was asked for.")
        } else {
            AttachmentSlice.Ok(body, mime, range.total)
        }
    }

    416 -> AttachmentSlice.Ok(ByteArray(0), mime, parseContentRange(contentRange)?.total)

    else -> refusal(status).let { (reason, missing) -> AttachmentSlice.Failed(reason, missing) }
}

internal fun refusal(status: Int, description: String = ""): Pair<String, Boolean> = when (status) {
    404 -> "The node no longer has this attachment." to true
    401, 403 -> "This node would not hand it over to this device." to false
    else -> "The node answered $status${if (description.isEmpty()) "" else " $description"}." to false
}

@Serializable
private class Written(val path: String, val bytes: Long)

private val lenient = Json { ignoreUnknownKeys = true }

fun uploadResultOf(status: Int, body: String): UploadResult = when (status) {
    200, 201 -> runCatching { lenient.decodeFromString<Written>(body) }.getOrNull()
        ?.let { UploadResult.Ok(it.path, it.bytes) }
        ?: UploadResult.Failed("The node took the file but did not say where it put it.")

    400 -> UploadResult.Failed("The node will not take a file by that name.")

    401, 403 -> UploadResult.Failed("This device may only read on that node.")

    404 -> UploadResult.Failed("That folder is gone, or the node cannot write into it.")

    408 -> UploadResult.Failed("Nothing arrived for a minute, so the node gave up on the upload.")

    413 -> UploadResult.Failed("The file is larger than that node will take.")

    else -> UploadResult.Failed("The node answered $status.")
}
