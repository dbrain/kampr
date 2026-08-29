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
        val title = note?.text("title") ?: "An agent needs you"
        val body = note?.text("body") ?: "Open Kampr to see which"
        val pane = note?.text("pane")
        // Nothing outstanding: every prompt this device was shown has been answered somewhere
        // else — at the desk, in the TUI, on another phone. Taking it down is the whole point of
        // the payload, and it is the one case that shows nothing at all.
        //
        // A payload with no `count` is a v1 node's, and v1 only ever sent news. `optInt` cannot
        // tell an absent field from a zero, so the field is read as text.
        if (note?.text("count") == "0") {
            clearBlockedNotification(applicationContext)
            return
        }
        // Likewise `alert`: absent means v1, and v1 was always news.
        when (note?.optBoolean("alert", true) != false) {
            true -> postBlockedNotification(applicationContext, title, body, pane)
            false -> postBlockedResync(applicationContext, title, body, pane)
        }
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
