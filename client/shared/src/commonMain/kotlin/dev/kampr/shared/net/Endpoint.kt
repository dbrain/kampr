package dev.kampr.shared.net

data class Endpoint(val baseUrl: String, val token: String? = null) {
    private val trimmed: String get() = baseUrl.trimEnd('/')

    val httpBase: String get() = trimmed

    val wsUrl: String
        get() = when {
            trimmed.startsWith("https://") -> "wss://" + trimmed.removePrefix("https://") + "/ws"
            trimmed.startsWith("http://") -> "ws://" + trimmed.removePrefix("http://") + "/ws"
            else -> "ws://$trimmed/ws"
        }

    val host: String
        get() = trimmed.substringAfter("://").substringBefore('/')

    val secure: Boolean get() = trimmed.startsWith("https://")

    // Exact spelling required: any other form fails the handshake outright.
    val subprotocol: String? get() = token?.let { "$TOKEN_SUBPROTOCOL_PREFIX$it" }

    companion object {
        const val TOKEN_SUBPROTOCOL_PREFIX = "kampr.token."
    }
}
