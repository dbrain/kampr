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

    suspend fun enrol(deviceName: String): PasskeyOutcome {
        val started = post(
            "/auth/webauthn/register/start",
            buildJsonObject {
                put("device_name", deviceName)
                // Which of the node's two option sets to state. Credential Manager cannot perform
                // the one a browser gets, and a phone sent it is offered a security key it has not
                // got.
                passkeys.platform?.let { put("platform", it) }
            },
        ) ?: return PasskeyOutcome.Refused("This node would not start a passkey registration.")
        val challengeId = started["challenge_id"]?.jsonPrimitive?.content ?: return MALFORMED
        val options = started["options"]?.toString() ?: return MALFORMED
        val credential = when (val result = passkeys.create(options)) {
            is PasskeyResult.Ok -> result.json
            PasskeyResult.Cancelled -> return PasskeyOutcome.Cancelled
            is PasskeyResult.Failed -> return PasskeyOutcome.Refused(explain(result.reason))
        }
        val finished = post(
            "/auth/webauthn/register/finish",
            buildJsonObject {
                put("challenge_id", challengeId)
                put("credential", json.parseToJsonElement(credential))
                put("device_name", deviceName)
            },
        ) ?: return PasskeyOutcome.Refused("This node did not accept that passkey. Nothing changed on it.")
        return enrolmentOf(finished)
    }

    suspend fun signIn(): PasskeyOutcome {
        val started = post("/auth/webauthn/authenticate/start", buildJsonObject { })
            ?: return PasskeyOutcome.Refused("This node has no passkey enrolled to sign in with.")
        val challengeId = started["challenge_id"]?.jsonPrimitive?.content ?: return MALFORMED
        val options = started["options"]?.toString() ?: return MALFORMED
        val credential = when (val result = passkeys.get(options)) {
            is PasskeyResult.Ok -> result.json
            PasskeyResult.Cancelled -> return PasskeyOutcome.Cancelled
            is PasskeyResult.Failed -> return PasskeyOutcome.Refused(explain(result.reason))
        }
        val finished = post(
            "/auth/webauthn/authenticate/finish",
            buildJsonObject {
                put("challenge_id", challengeId)
                put("credential", json.parseToJsonElement(credential))
            },
        ) ?: return PasskeyOutcome.Refused("That passkey was not accepted by this node.")
        return enrolmentOf(finished)
    }

    // The authenticator says what it refused; only the node can say *why* it was ever going to.
    // Fetched from the same address this client dials, which is the half of the check the phone
    // can do — `passkeyRefusal` owns what each answer means.
    private suspend fun explain(reason: String): String {
        val identity = passkeys.identity ?: return reason
        val document = runCatching {
            val response = client.get("${endpoint.httpBase}$ASSET_LINKS_PATH")
            if (response.status.isSuccess()) response.bodyAsText() else null
        }.getOrNull()
        return passkeyRefusal(document, identity, endpoint.host, reason)
    }

    private fun enrolmentOf(body: JsonObject): PasskeyOutcome {
        val token = body["token"]?.jsonPrimitive?.content ?: return MALFORMED
        val device = body["device"] as? JsonObject
        return PasskeyOutcome.Enrolled(
            Enrolment(
                token = token,
                deviceId = device?.get("id")?.jsonPrimitive?.content,
                name = device?.get("name")?.jsonPrimitive?.content,
                role = device?.get("role")?.jsonPrimitive?.content,
            )
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

    private companion object {
        val MALFORMED = PasskeyOutcome.Refused("This node answered with something that is not a WebAuthn ceremony.")
    }
}

// Three endings, and only one of them is worth a message: a cancelled ceremony is somebody
// changing their mind, and an error strip for that is noise about a decision they made.
sealed interface PasskeyOutcome {
    data class Enrolled(val enrolment: Enrolment) : PasskeyOutcome

    data object Cancelled : PasskeyOutcome

    data class Refused(val reason: String) : PasskeyOutcome
}
