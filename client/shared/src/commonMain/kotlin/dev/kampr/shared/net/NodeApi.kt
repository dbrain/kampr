package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.parameter
import io.ktor.client.statement.bodyAsText
import dev.kampr.shared.util.parseHttpDateMillis
import io.ktor.http.HttpStatusCode
import io.ktor.http.isSuccess
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

// `hello.security` decides what a client may offer; this is the same answer without a socket, and
// it carries the one thing the socket does not: the canonical origin a second device is pointed at.
@Serializable
data class NodeOrigin(
    val origin: String = "",
    val tier: Int = 0,
    val passkeys: Boolean = false,
    val installable: Boolean = false,
)

@Serializable
data class NodeStatus(
    @SerialName("node_id") val nodeId: String = "",
    @SerialName("node_name") val nodeName: String = "",
    val build: String? = null,
    val protocol: Int = 0,
    val bundle: Boolean = false,
    val enrolled: Boolean = false,
    val security: NodeOrigin = NodeOrigin(),
)

class NodeApi(private val client: HttpClient, private val endpoint: Endpoint) {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    suspend fun info(): NodeStatus? = runCatching {
        val response = client.get("${endpoint.httpBase}/api/node") { auth() }
        if (response.status.isSuccess()) {
            json.decodeFromString(NodeStatus.serializer(), response.bodyAsText())
        } else {
            null
        }
    }.getOrNull()

    // Half the round trip is the best correction available without a timestamp on `hello`, and it
    // is worth having: everything time-shaped in this client compares one clock against the other.
    suspend fun clockOffsetMillis(): Double? = runCatching {
        val sent = wallClockMillis()
        val response = client.get("${endpoint.httpBase}/api/node") { auth() }
        val here = (wallClockMillis() + sent) / 2.0
        parseHttpDateMillis(response.headers["Date"])?.minus(here)
    }.getOrNull()

    // The service worker caches `/api/node` and `/api/warm` behind a push and serves them back to
    // the page — a loop with no entrance until somebody asks for them. This is the entrance: a
    // herd painted from the warm cache before the socket has finished opening.
    suspend fun warm(pane: String? = null): String? = runCatching {
        val response = client.get("${endpoint.httpBase}/api/warm") {
            auth()
            pane?.let { parameter("pane", it) }
        }
        if (response.status.isSuccess()) response.bodyAsText() else null
    }.getOrNull()

    // A browser is never shown the status of a failed WebSocket handshake, so on the socket alone a
    // node that has forgotten this device is indistinguishable from one that is switched off. Over
    // plain HTTP the same token gets a status code. Only an explicit 401 counts: a node that is
    // down answers nothing, and reporting that as a refusal would send the operator to re-pair
    // against something that was going to come back on its own.
    suspend fun refusesToken(): Boolean = runCatching {
        client.get("${endpoint.httpBase}/api/devices") { auth() }.status == HttpStatusCode.Unauthorized
    }.getOrDefault(false)

    private fun io.ktor.client.request.HttpRequestBuilder.auth() {
        endpoint.token?.let { header("Authorization", "Bearer $it") }
    }
}
