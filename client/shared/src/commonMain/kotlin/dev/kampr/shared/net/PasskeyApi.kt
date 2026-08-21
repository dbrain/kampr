package dev.kampr.shared.net

import io.ktor.client.HttpClient
import io.ktor.client.request.header
import io.ktor.client.request.post
import io.ktor.client.request.setBody
import io.ktor.client.statement.bodyAsText
import io.ktor.http.ContentType
import io.ktor.http.contentType
import io.ktor.http.isSuccess
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put

// The four `/auth/webauthn/*` routes have existed and worked since phase 3 with not one caller in
// the client, in `kamprWeb.js` or in either `.wasm`. This is the caller.
//
// Both halves end in an `Enrolment`, because both mint a device: registering a passkey enrols the
// device that holds it, and signing in with one enrols the device that presented it. That is why
// neither is reachable below tier 1 — a WebAuthn RP ID must be a registrable domain, and an IP
// address is never one.
class PasskeyApi(
    private val client: HttpClient,
    private val endpoint: Endpoint,
    private val passkeys: Passkeys = createPasskeys(),
) {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    val available: Boolean get() = passkeys.available

    suspend fun enrol(deviceName: String): Enrolment? {
        val started = post("/auth/webauthn/register/start", buildJsonObject { put("device_name", deviceName) })
            ?: return null
        val challengeId = started["challenge_id"]?.jsonPrimitive?.content ?: return null
        val options = started["options"]?.toString() ?: return null
        val credential = passkeys.create(options) ?: return null
        val finished = post(
            "/auth/webauthn/register/finish",
            buildJsonObject {
                put("challenge_id", challengeId)
                put("credential", json.parseToJsonElement(credential))
                put("device_name", deviceName)
            },
        ) ?: return null
        return enrolmentOf(finished)
    }

    suspend fun signIn(): Enrolment? {
        val started = post("/auth/webauthn/authenticate/start", buildJsonObject { }) ?: return null
        val challengeId = started["challenge_id"]?.jsonPrimitive?.content ?: return null
        val options = started["options"]?.toString() ?: return null
        val credential = passkeys.get(options) ?: return null
        val finished = post(
            "/auth/webauthn/authenticate/finish",
            buildJsonObject {
                put("challenge_id", challengeId)
                put("credential", json.parseToJsonElement(credential))
            },
        ) ?: return null
        return enrolmentOf(finished)
    }

    private fun enrolmentOf(body: JsonObject): Enrolment? {
        val token = body["token"]?.jsonPrimitive?.content ?: return null
        val device = body["device"] as? JsonObject
        return Enrolment(
            token = token,
            deviceId = device?.get("id")?.jsonPrimitive?.content,
            name = device?.get("name")?.jsonPrimitive?.content,
            role = device?.get("role")?.jsonPrimitive?.content,
        )
    }

    private suspend fun post(path: String, body: JsonObject): JsonObject? = runCatching {
        val response = client.post("${endpoint.httpBase}$path") {
            endpoint.token?.let { header("Authorization", "Bearer $it") }
            contentType(ContentType.Application.Json)
            setBody(body.toString())
        }
        if (response.status.isSuccess()) json.parseToJsonElement(response.bodyAsText()).jsonObject else null
    }.getOrNull()
}
