package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.statement.readRawBytes
import io.ktor.http.HttpStatusCode
import io.ktor.http.encodeURLPath
import io.ktor.http.encodeURLPathPart

// Bytes or a sentence. A blank box where a picture should be is the defect this whole surface is
// most likely to grow, so there is nowhere here for a failure to become an empty success.
sealed interface AttachmentBytes {
    class Ok(val bytes: ByteArray, val mime: String?) : AttachmentBytes
    data class Failed(val reason: String) : AttachmentBytes
}

// A pane id carries a slash of its own — `<node>/<pane>` — and it is left as a slash: the route is
// `/api/attachment/{pane}/{id}` with the pane spelled the way every other frame spells it.
class AttachmentApi(private val client: HttpClient, private val endpoint: Endpoint) {
    suspend fun fetch(pane: String, id: String): AttachmentBytes {
        val url = "${endpoint.httpBase}/api/attachment/${pane.encodeURLPath()}/${id.encodeURLPathPart()}"
        val response = runCatching {
            client.get(url) { endpoint.token?.let { header("Authorization", "Bearer $it") } }
        }.getOrElse { failure ->
            return AttachmentBytes.Failed(
                "Could not reach the node: ${failure.message ?: "the request went nowhere"}."
            )
        }
        return when (response.status) {
            HttpStatusCode.OK -> {
                val bytes = runCatching { response.readRawBytes() }.getOrNull()
                if (bytes == null || bytes.isEmpty()) {
                    AttachmentBytes.Failed("The node answered with no bytes at all.")
                } else {
                    AttachmentBytes.Ok(bytes, response.headers["Content-Type"]?.substringBefore(';')?.trim())
                }
            }
            HttpStatusCode.NotFound ->
                AttachmentBytes.Failed("The node no longer has this attachment.")
            HttpStatusCode.Unauthorized, HttpStatusCode.Forbidden ->
                AttachmentBytes.Failed("This node would not hand it over to this device.")
            else ->
                AttachmentBytes.Failed("The node answered ${response.status.value} ${response.status.description}.")
        }
    }
}
