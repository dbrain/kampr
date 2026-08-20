package dev.kampr.shared.net

import dev.kampr.shared.push.PushEnrolment
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

// What the node says about notifications. `available` is the only field a client branches on:
// it is false on a Tier 0 origin, false when the operator turned push off, and false when no
// VAPID key could be loaded — three reasons a subscribe button must be absent rather than
// present and failing at the last step.
@Serializable
data class PushState(
    val available: Boolean = false,
    val key: String? = null,
    @SerialName("secure_context") val secureContext: Boolean = false,
    val unlocks: List<String> = emptyList(),
    val subscribed: Boolean = false,
    val rules: List<PushRule> = emptyList(),
)

@Serializable
data class PushRule(
    @SerialName("pane_id") val paneId: String,
    val muted: Boolean = false,
    @SerialName("snooze_until") val snoozeUntil: Long? = null,
)

@Serializable
private data class RuleList(val rules: List<PushRule> = emptyList())

// The wildcard: a rule that covers every agent on this device.
const val ALL_PANES = "*"

class PushApi(private val client: HttpClient, private val endpoint: Endpoint) {
    private val json = Json { ignoreUnknownKeys = true; isLenient = true; encodeDefaults = true }

    suspend fun state(): PushState? = runCatching {
        json.decodeFromString(
            PushState.serializer(),
            client.get("${endpoint.httpBase}/api/push") { auth() }.bodyAsText(),
        )
    }.getOrNull()

    suspend fun subscribe(enrolment: PushEnrolment): Boolean = runCatching {
        client.post("${endpoint.httpBase}/api/push/subscribe") {
            auth()
            contentType(ContentType.Application.Json)
            setBody(json.encodeToString(SubscribeBody.serializer(), SubscribeBody.of(enrolment)))
        }.status.isSuccess()
    }.getOrDefault(false)

    suspend fun unsubscribe(endpointUrl: String): Boolean = runCatching {
        client.post("${endpoint.httpBase}/api/push/unsubscribe") {
            auth()
            contentType(ContentType.Application.Json)
            setBody(json.encodeToString(UnsubscribeBody.serializer(), UnsubscribeBody(endpointUrl)))
        }.status.isSuccess()
    }.getOrDefault(false)

    suspend fun rule(rule: PushRule): List<PushRule>? = runCatching {
        val body = client.post("${endpoint.httpBase}/api/push/rules") {
            auth()
            contentType(ContentType.Application.Json)
            setBody(json.encodeToString(PushRule.serializer(), rule))
        }.bodyAsText()
        json.decodeFromString(RuleList.serializer(), body).rules
    }.getOrNull()

    private fun io.ktor.client.request.HttpRequestBuilder.auth() {
        endpoint.token?.let { header("Authorization", "Bearer $it") }
    }
}

@Serializable
private data class SubscribeKeys(val p256dh: String, val auth: String)

@Serializable
private data class SubscribeBody(val endpoint: String, val kind: String, val keys: SubscribeKeys) {
    companion object {
        fun of(e: PushEnrolment) = SubscribeBody(e.endpoint, e.kind, SubscribeKeys(e.p256dh, e.auth))
    }
}

@Serializable
private data class UnsubscribeBody(val endpoint: String)
