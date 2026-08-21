package dev.kampr.app

import dev.kampr.shared.push.UNIFIED_PUSH_INSTANCE
import dev.kampr.shared.push.UnifiedPushEndpoints
import org.json.JSONObject
import org.unifiedpush.android.connector.FailedReason
import org.unifiedpush.android.connector.PushService
import org.unifiedpush.android.connector.data.PushEndpoint
import org.unifiedpush.android.connector.data.PushMessage

// The Android half of the decision on record (docs/08-notifications.md): UnifiedPush, not FCM.
// A distributor's endpoint is an RFC 8291 endpoint, so the node's sender is unchanged — no Google
// project, no per-app secret, which is the whole point for somebody self-hosting a terminal bridge.
class KamprPushService : PushService() {
    // A distributor rotates endpoints on its own schedule, with no app on screen. Posting it from
    // here rather than from a screen is what stops the device simply going quiet.
    override fun onNewEndpoint(endpoint: PushEndpoint, instance: String) {
        if (instance != UNIFIED_PUSH_INSTANCE) return
        val keys = endpoint.pubKeySet
        UnifiedPushEndpoints.arrived(applicationContext, endpoint.url, keys?.pubKey, keys?.auth)
    }

    override fun onMessage(message: PushMessage, instance: String) {
        if (instance != UNIFIED_PUSH_INSTANCE) return
        // A distributor's endpoint is a URL anyone who learns it can POST to, so the only thing
        // that makes a payload this node's is that it decrypted under this device's own key. An
        // undecryptable one is a stranger writing on the lock screen.
        if (!message.decrypted) return
        val note = runCatching { JSONObject(message.content.decodeToString()) }.getOrNull()
        postBlockedNotification(
            applicationContext,
            note?.text("title") ?: "An agent needs you",
            note?.text("body") ?: "Open Kampr to see which",
            note?.text("pane"),
        )
    }

    override fun onRegistrationFailed(reason: FailedReason, instance: String) {
        UnifiedPushEndpoints.forget(applicationContext)
    }

    override fun onUnregistered(instance: String) {
        UnifiedPushEndpoints.forget(applicationContext)
    }

    private fun JSONObject.text(key: String): String? =
        optString(key).takeIf { it.isNotBlank() && it != "null" }
}
