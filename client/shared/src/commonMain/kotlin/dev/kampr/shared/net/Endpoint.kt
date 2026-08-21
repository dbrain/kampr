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

// A host a scanner is allowed to hand back: a name, an address literal, or a bracketed IPv6, with
// an optional port. Deliberately narrow — a camera decodes whatever is in front of it, and a Wi-Fi
// card, a payment code and a noticeboard poster are all QR symbols too.
private val SCANNED_HOST = Regex("""^(\[[0-9A-Fa-f:]{2,45}]|[A-Za-z0-9](?:[A-Za-z0-9._-]{0,252}[A-Za-z0-9])?)(:\d{1,5})?$""")

// A pairing code is what the node prints: six characters of its own alphabet, or the four-and-four
// form the CLI uses. Anything longer is not a code and is not going in a URL this app then dials.
private val SCANNED_CODE = Regex("""^[A-Za-z0-9-]{4,32}$""")

// What a scanned QR turns into. The desktop's symbol is `origin#pair=<code>` and the code rides in
// the fragment on purpose — a fragment is never sent, so it reaches neither the node's access log
// nor the proxy's. Null means "that was not a Kampr node", which is the common case for a camera.
fun pairingFrom(scanned: String): Endpoint? {
    val text = scanned.trim()
    if (text.isEmpty() || text.any { it.isWhitespace() || it.code < 0x20 }) return null
    val (address, fragment) = text.substringBefore('#') to text.substringAfter('#', "")
    val scheme = address.substringBefore("://", "")
    if (scheme.isNotEmpty() && scheme != "http" && scheme != "https") return null
    val rest = (if (scheme.isEmpty()) address else address.substringAfter("://")).trimEnd('/')
    if (!SCANNED_HOST.matches(rest)) return null
    val code = fragment
        .split('&')
        .mapNotNull { it.split('=', limit = 2).takeIf { parts -> parts.size == 2 } }
        .firstOrNull { it[0] == "pair" }
        ?.get(1)
    if (fragment.isNotEmpty() && code == null) return null
    if (code != null && !SCANNED_CODE.matches(code)) return null
    return endpointFrom(if (scheme.isEmpty()) rest else "$scheme://$rest", code)
}
