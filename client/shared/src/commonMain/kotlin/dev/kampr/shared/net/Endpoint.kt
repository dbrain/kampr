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

private val IPV4 = Regex("""^\d{1,3}(\.\d{1,3}){3}$""")

// A name that can hold a public certificate takes https; anything that cannot — an address
// literal, a single-label LAN name, an mDNS name — takes http. The panel prints the result back,
// so a wrong guess is one the operator can see and correct rather than one that just fails.
private fun schemeFor(hostPort: String): String {
    val host = if (hostPort.startsWith("[")) hostPort.substringBefore(']').removePrefix("[")
    else hostPort.substringBefore(':')
    val lan = ':' in host ||
        IPV4.matches(host) ||
        '.' !in host ||
        host.endsWith(".local", ignoreCase = true)
    return if (lan) "http://" else "https://"
}

// What somebody reads off `kampr init` is a host and a port. Requiring the scheme as well is a
// step that only ever goes wrong, so a bare `host:port` is completed here.
fun endpointFrom(typed: String, code: String? = null): Endpoint? {
    val text = typed.trim()
    if (text.isEmpty()) return null
    val scheme = text.substringBefore("://", "")
    val rest = if (scheme.isEmpty()) text else text.substringAfter("://")
    val body = rest.trimEnd('/')
    if (body.isEmpty()) return null
    val prefix = if (scheme.isEmpty()) schemeFor(body) else "$scheme://"
    return Endpoint(prefix + body, code?.trim()?.takeIf { it.isNotEmpty() })
}
