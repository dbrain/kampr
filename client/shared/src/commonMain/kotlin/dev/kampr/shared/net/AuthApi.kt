package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.request.get
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.contentType
import io.ktor.http.isSuccess
import kotlinx.serialization.SerialName
import kotlinx.serialization.Serializable
import kotlinx.serialization.json.Json

// Only what hello.security cannot supply: where to point a phone, and the pairing code.
// Tier, encryption and passkey availability are decided from hello.security, never from here.
@Serializable
data class SetupStatus(
    val address: String,
    @SerialName("pairing_code") val pairingCode: String? = null,
    val devices: Int = 0,
    val version: String? = null,
)

@Serializable
data class DeviceRecord(
    val id: String,
    val name: String,
    val kind: String = "browser",
    val role: String = "full",
    @SerialName("added_at") val addedAt: String? = null,
    @SerialName("last_seen") val lastSeen: String? = null,
    val current: Boolean = false,
)

@Serializable
private data class DeviceList(val devices: List<DeviceRecord> = emptyList())

@Serializable
private data class PairResult(val token: String? = null)

// The node's auth endpoints are specified alongside brief B; this is the client half of that
// contract and every call degrades to null rather than blocking the herd.
class AuthApi(private val client: HttpClient, private val endpoint: Endpoint) {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    suspend fun status(): SetupStatus? = call("/auth/status") { json.decodeFromString(SetupStatus.serializer(), it) }

    suspend fun devices(): List<DeviceRecord> =
        call("/auth/devices") { json.decodeFromString(DeviceList.serializer(), it).devices } ?: emptyList()

    // Ktor does not throw on a non-2xx, so without this a revocation that the node refused —
    // or never routed — reports success and the device stays in the list, still connected.
    suspend fun revoke(id: String): Boolean = runCatching {
        client.post("${endpoint.httpBase}/auth/devices/$id/revoke") { auth() }.status.isSuccess()
    }.getOrDefault(false)

    // The enrolment endpoints are specified alongside brief B. A node that returns a device
    // token is used as such; one that does not leaves the entered code to stand as the token,
    // so a simpler node still pairs rather than failing closed.
    suspend fun pair(code: String): String? = runCatching {
        val body = client.post("${endpoint.httpBase}/auth/pair") {
            contentType(ContentType.Application.Json)
            setBody("{\"code\":\"$code\"}")
        }.bodyAsText()
        json.decodeFromString(PairResult.serializer(), body).token
    }.getOrNull()

    private suspend fun <T> call(path: String, parse: (String) -> T): T? = runCatching {
        parse(client.get("${endpoint.httpBase}$path") { auth() }.bodyAsText())
    }.getOrNull()

    private fun io.ktor.client.request.HttpRequestBuilder.auth() {
        endpoint.token?.let { header("Authorization", "Bearer $it") }
    }
}
