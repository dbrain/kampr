package dev.kampr.shared.net

// WebAuthn, as the one thing a platform either has or has not. A passkey button that cannot work
// must be absent rather than present-and-failing, and there are two independent reasons it might
// not: the origin cannot carry a WebAuthn RP ID (an IP address never can), which `hello.security`
// answers, and this platform has no authenticator API at all, which only the platform can answer.
interface Passkeys {
    val available: Boolean

    // The node hands over `webauthn-rs`'s own challenge JSON and takes back the credential JSON
    // its `RegisterPublicKeyCredential` decodes. Neither shape is this client's to invent, so both
    // cross this seam as opaque JSON.
    suspend fun create(optionsJson: String): String?

    suspend fun get(optionsJson: String): String?
}

expect fun createPasskeys(): Passkeys

class NoPasskeys : Passkeys {
    override val available: Boolean = false
    override suspend fun create(optionsJson: String): String? = null
    override suspend fun get(optionsJson: String): String? = null
}
