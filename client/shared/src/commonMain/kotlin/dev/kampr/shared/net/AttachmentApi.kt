package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.request.HttpRequestBuilder
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.prepareGet
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsChannel
import io.ktor.client.statement.bodyAsText
import io.ktor.client.statement.readRawBytes
import io.ktor.http.ContentType
import io.ktor.http.HttpHeaders
import io.ktor.http.HttpStatusCode
import io.ktor.http.content.OutgoingContent
import io.ktor.http.encodeURLParameter
import io.ktor.http.encodeURLPath
import io.ktor.http.encodeURLPathPart
import io.ktor.utils.io.ByteWriteChannel
import io.ktor.utils.io.readAvailable
import io.ktor.utils.io.writeFully
import kotlinx.coroutines.CancellationException

// Bytes or a sentence. A blank box where a picture should be is the defect this whole surface is
// most likely to grow, so there is nowhere here for a failure to become an empty success.
sealed interface AttachmentBytes {
    class Ok(val bytes: ByteArray, val mime: String?) : AttachmentBytes
    data class Failed(val reason: String, val missing: Boolean = false) : AttachmentBytes
}

private const val CHUNK = 256 * 1024

private fun unreachable(failure: Throwable) = "Could not reach the node: ${failure.message ?: "the request went nowhere"}."

// A pane id carries a slash of its own — `<node>/<pane>` — and it is left as a slash: the route is
// `/api/attachment/{pane}/{id}` with the pane spelled the way every other frame spells it.
class AttachmentApi(private val client: HttpClient, private val endpoint: Endpoint) {
    private fun url(pane: String, id: String) = "${endpoint.httpBase}/api/attachment/${pane.encodeURLPath()}/${id.encodeURLPathPart()}"

    private fun HttpRequestBuilder.bearer() {
        endpoint.token?.let { header(HttpHeaders.Authorization, "Bearer $it") }
    }

    suspend fun fetch(pane: String, id: String): AttachmentBytes {
        val response = runCatching { client.get(url(pane, id)) { bearer() } }
            .getOrElse { failure -> return AttachmentBytes.Failed(unreachable(failure)) }
        if (response.status != HttpStatusCode.OK) {
            val (reason, missing) = refusal(response.status.value, response.status.description)
            return AttachmentBytes.Failed(reason, missing)
        }
        val bytes = runCatching { response.readRawBytes() }.getOrNull()
        return if (bytes == null || bytes.isEmpty()) {
            AttachmentBytes.Failed("The node answered with no bytes at all.")
        } else {
            AttachmentBytes.Ok(bytes, response.mime())
        }
    }

    suspend fun range(pane: String, id: String, start: Long, endInclusive: Long): AttachmentSlice {
        val response = runCatching {
            client.get(url(pane, id)) {
                bearer()
                header(HttpHeaders.Range, rangeHeader(start, endInclusive))
            }
        }.getOrElse { failure -> return AttachmentSlice.Failed(unreachable(failure)) }
        val body = when (response.status.value) {
            200, 206 -> runCatching { response.readRawBytes() }.getOrElse { failure -> return AttachmentSlice.Failed(unreachable(failure)) }
            else -> ByteArray(0)
        }
        return sliceOf(response.status.value, response.headers[HttpHeaders.ContentRange], response.mime(), body, start, endInclusive)
    }

    // Chunk by chunk from the socket into the sink: the whole file never sits in memory, which is
    // what a download of anything larger than a phone's heap needs.
    suspend fun download(pane: String, id: String, sink: (mime: String?) -> DownloadSink?): String? = try {
        client.prepareGet(url(pane, id)) { bearer() }.execute { response ->
            if (response.status != HttpStatusCode.OK) return@execute null
            val target = sink(response.mime()) ?: return@execute null
            val channel = response.bodyAsChannel()
            val buffer = ByteArray(CHUNK)
            try {
                while (true) {
                    val n = channel.readAvailable(buffer, 0, buffer.size)
                    if (n < 0) break
                    if (n > 0) target.write(buffer, n)
                }
                target.finish()
            } catch (failure: Throwable) {
                target.abort()
                throw failure
            }
        }
    } catch (cancelled: CancellationException) {
        throw cancelled
    } catch (_: Throwable) {
        null
    }

    suspend fun upload(pane: String, dir: String, name: String, source: UploadSource, progress: (Long) -> Unit): UploadResult {
        val url = "${endpoint.httpBase}/api/upload/${pane.encodeURLPath()}" +
            "?dir=${dir.encodeURLParameter()}&name=${name.encodeURLParameter()}"
        if (source is DirectUpload) {
            val answer = source.post(url, endpoint.token, progress) ?: return UploadResult.Failed("Could not reach the node.")
            return uploadResultOf(answer.status, answer.body)
        }
        val response = try {
            client.post(url) {
                bearer()
                setBody(Streamed(source, progress))
            }
        } catch (cancelled: CancellationException) {
            throw cancelled
        } catch (failure: Throwable) {
            return UploadResult.Failed(unreachable(failure))
        } finally {
            source.close()
        }
        return uploadResultOf(response.status.value, runCatching { response.bodyAsText() }.getOrDefault(""))
    }
}

private fun io.ktor.client.statement.HttpResponse.mime() = headers[HttpHeaders.ContentType]?.substringBefore(';')?.trim()

private class Streamed(private val source: UploadSource, private val progress: (Long) -> Unit) : OutgoingContent.WriteChannelContent() {
    override val contentLength: Long? get() = source.size
    override val contentType: ContentType get() = ContentType.Application.OctetStream

    override suspend fun writeTo(channel: ByteWriteChannel) {
        val buffer = ByteArray(CHUNK)
        var sent = 0L
        while (true) {
            val n = source.read(buffer)
            if (n < 0) break
            channel.writeFully(buffer, 0, n)
            sent += n
            progress(sent)
        }
    }
}
