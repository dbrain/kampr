package dev.kampr.shared.net

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.jsonObject

// WebAuthn, as the one thing a platform either has or has not. A passkey button that cannot work
// must be absent rather than present-and-failing, and there are two independent reasons it might
// not: the origin cannot carry a WebAuthn RP ID (an IP address never can), which `hello.security`
// answers, and this platform has no authenticator API at all, which only the platform can answer.
interface Passkeys {
    val available: Boolean

    // What `platform` on `/auth/webauthn/register/start` is told. The node states two option sets
    // and only one of them is a ceremony Credential Manager can perform, so the client that is
    // going to run it says which it is. `null` is a browser.
    val platform: String?
        get() = null

    // This app's package and signing certificate — the pair a relying party has to name in its
    // asset links before Android will let it hold a passkey. `null` wherever apps are not a thing.
    val identity: AppIdentity?
        get() = null

    // The node hands over `webauthn-rs`'s own challenge JSON and takes back the credential JSON
    // its `RegisterPublicKeyCredential` decodes. Neither shape is this client's to invent, so both
    // cross this seam as opaque JSON.
    suspend fun create(optionsJson: String): PasskeyResult

    suspend fun get(optionsJson: String): PasskeyResult
}

// A refusal and a change of mind are not the same event: backing out of the system sheet must
// leave nothing on screen, and everything else has to say what went wrong or it is a shrug.
sealed interface PasskeyResult {
    data class Ok(val json: String) : PasskeyResult

    data object Cancelled : PasskeyResult

    data class Failed(val reason: String) : PasskeyResult
}

data class AppIdentity(val packageName: String, val fingerprint: String)

expect fun createPasskeys(): Passkeys

class NoPasskeys : Passkeys {
    override val available: Boolean = false
    override suspend fun create(optionsJson: String): PasskeyResult = UNAVAILABLE
    override suspend fun get(optionsJson: String): PasskeyResult = UNAVAILABLE

    private companion object {
        val UNAVAILABLE = PasskeyResult.Failed("This build has no authenticator to ask.")
    }
}

private val json = Json { ignoreUnknownKeys = true; isLenient = true }

// A browser takes `{"publicKey": {…}}` and hands the inner object to `navigator.credentials`.
// Credential Manager takes the inner object itself, so the unwrapping that happens inside the
// browser has to happen here instead.
fun credentialManagerRequest(optionsJson: String): String? {
    val key = runCatching { json.parseToJsonElement(optionsJson).jsonObject }
        .getOrNull()
        ?.get("publicKey") as? JsonObject
        ?: return null
    if (key["challenge"] == null) return null
    return key.toString()
}
