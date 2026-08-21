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

// Composed, not fetched: no route serves this shape, and inventing one would have been a second
// spelling of `/api/node` and `/api/devices`. `address` is the node's own canonical origin — the
// URL a second device is pointed at, which the socket never carries.
data class SetupStatus(
    val address: String,
    val devices: Int = 0,
    val version: String? = null,
    // The same two answers `hello.security` gives, from the route that needs no token — which is
    // the only way a device with no token yet can be offered the passkey that would get it one.
    val passkeys: Boolean = false,
    val installable: Boolean = false,
)

// The node's `Device`, field for field. `created_at`/`last_seen_at` are epoch seconds, not
// strings — the shape this decoded into before named two columns that have never been sent.
@Serializable
data class DeviceRecord(
    val id: String,
    val name: String,
    val role: String = "readonly",
    @SerialName("created_at") val createdAt: Long? = null,
    @SerialName("last_seen_at") val lastSeenAt: Long? = null,
    @SerialName("expires_at") val expiresAt: Long? = null,
    @SerialName("revoked_at") val revokedAt: Long? = null,
    @SerialName("user_agent") val userAgent: String? = null,
    val origin: String? = null,
) {
    // The node returns revoked rows too, and expiry is the node's clock to judge, not a phone's.
    val active: Boolean get() = revokedAt == null
}

// `/auth/pair` answers with the device it just enrolled, and that id is the only way a client can
// tell itself apart from the others in the list — a device that cannot recognise itself offers
// "Revoke" against the connection it is using.
data class Enrolment(val token: String, val deviceId: String?, val name: String?, val role: String?)

@Serializable
private data class DeviceList(val devices: List<DeviceRecord> = emptyList())

@Serializable
private data class PairResult(val token: String? = null, val device: DeviceRecord? = null)

@Serializable
private data class PairingOffer(val code: String, val role: String = "full", @SerialName("expires_in") val expiresIn: Long = 0)

@Serializable
private data class RedeemRequest(val code: String, @SerialName("device_name") val deviceName: String? = null)

// Every route here is one the node actually serves. It used to call `/auth/status`,
// `/auth/devices` and `/auth/devices/{id}/revoke` — two of which the SPA fallback answered with
// 200 and a page of HTML, and the third with 405, all of it swallowed by `getOrNull`.
class AuthApi(
    private val client: HttpClient,
    private val endpoint: Endpoint,
    private val node: NodeApi = NodeApi(client, endpoint),
) {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true; encodeDefaults = true }

    suspend fun status(): SetupStatus? {
        val info = node.info() ?: return null
        return SetupStatus(
            address = info.security.origin.ifBlank { endpoint.httpBase },
            devices = devices().count { it.active },
            version = info.build,
            passkeys = info.security.passkeys,
            installable = info.security.installable,
        )
    }

    suspend fun devices(): List<DeviceRecord> =
        get("/api/devices") { json.decodeFromString(DeviceList.serializer(), it).devices } ?: emptyList()

    // Ktor does not throw on a non-2xx, so without this a revocation the node refused — or never
    // routed — reports success and the device stays in the list, still connected.
    suspend fun revoke(id: String): Boolean = runCatching {
        client.post("${endpoint.httpBase}/api/devices/$id/revoke") { auth() }.status.isSuccess()
    }.getOrDefault(false)

    // A code minted by an already-enrolled device is armed by construction, so the browser wizard
    // can print one that redeems as it stands. A code printed at a console cannot: `kampr setup`
    // is a herdr popup and a read-only device sees every frame of it.
    suspend fun pairingCode(): String? = runCatching {
        val response = client.post("${endpoint.httpBase}/api/pair") { auth() }
        if (!response.status.isSuccess()) null
        else json.decodeFromString(PairingOffer.serializer(), response.bodyAsText()).code
    }.getOrNull()

    // A refused code is a refusal, not a token. Treating the typed code as a bearer is what turned
    // a mistyped pairing code into an endless `auth.rejected` loop with nothing on screen.
    suspend fun pair(code: String, deviceName: String? = null): Enrolment? = runCatching {
        val response = client.post("${endpoint.httpBase}/auth/pair") {
            contentType(ContentType.Application.Json)
            setBody(json.encodeToString(RedeemRequest.serializer(), RedeemRequest(code, deviceName)))
        }
        if (!response.status.isSuccess()) {
            null
        } else {
            val result = json.decodeFromString(PairResult.serializer(), response.bodyAsText())
            result.token?.let { Enrolment(it, result.device?.id, result.device?.name, result.device?.role) }
        }
    }.getOrNull()

    private suspend fun <T> get(path: String, parse: (String) -> T): T? = runCatching {
        val response = client.get("${endpoint.httpBase}$path") { auth() }
        if (response.status.isSuccess()) parse(response.bodyAsText()) else null
    }.getOrNull()

    private fun io.ktor.client.request.HttpRequestBuilder.auth() {
        endpoint.token?.let { header("Authorization", "Bearer $it") }
    }
}
