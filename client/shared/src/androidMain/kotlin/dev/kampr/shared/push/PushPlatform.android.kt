package dev.kampr.shared.push

import android.Manifest
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import dev.kampr.shared.platform.KamprAndroid
import dev.kampr.shared.net.Endpoint
import dev.kampr.shared.net.PushApi
import dev.kampr.shared.net.createHttpClient
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.async
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.launch
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.filterNotNull
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.withTimeoutOrNull
import org.unifiedpush.android.connector.UnifiedPush

// One registration per app, and this is its name. The distributor keys its own storage on it, so
// changing it orphans whatever endpoint is live.
const val UNIFIED_PUSH_INSTANCE = "kampr"

private const val PREFS = "kampr"
private const val KEY_ENDPOINT = "push.endpoint"
private const val KEY_P256DH = "push.p256dh"
private const val KEY_AUTH = "push.auth"
private const val ENDPOINT_TIMEOUT_MS = 30_000L

// The same two keys `AppState` writes through `Prefs`, read here because this runs when no page
// does and there is no `AppState` to ask.
private const val KEY_NODE = "endpoint"
private const val KEY_TOKEN = "token"

private val reporting = CoroutineScope(SupervisorJob() + Dispatchers.IO)

// An endpoint arrives from the distributor asynchronously, through a Service, possibly long after
// the screen that asked for it has gone. This is where the two meet: the service publishes, the
// subscribe call awaits, and the last one is kept on disk so a restart still knows what to
// unsubscribe.
object UnifiedPushEndpoints {
    private val latest = MutableStateFlow<PushEnrolment?>(null)

    // Remember it, and tell the node. A distributor rotates endpoints on its own schedule with
    // no app on screen, and a device whose node still holds the old one simply goes quiet.
    fun arrived(context: Context, url: String, p256dh: String?, auth: String?) {
        val enrolment = PushEnrolment(url, p256dh.orEmpty(), auth.orEmpty(), kind = "unifiedpush")
        store(context).edit()
            .putString(KEY_ENDPOINT, url)
            .putString(KEY_P256DH, p256dh)
            .putString(KEY_AUTH, auth)
            .apply()
        latest.value = enrolment
        report(context, enrolment)
    }

    private fun report(context: Context, enrolment: PushEnrolment) {
        val prefs = store(context)
        val base = prefs.getString(KEY_NODE, null) ?: return
        val token = prefs.getString(KEY_TOKEN, null) ?: return
        reporting.launch {
            val client = createHttpClient()
            try {
                PushApi(client, Endpoint(base, token)).subscribe(enrolment)
            } finally {
                client.close()
            }
        }
    }

    fun forget(context: Context) {
        store(context).edit().remove(KEY_ENDPOINT).remove(KEY_P256DH).remove(KEY_AUTH).apply()
        latest.value = null
    }

    fun saved(context: Context): PushEnrolment? {
        val prefs = store(context)
        val url = prefs.getString(KEY_ENDPOINT, null) ?: return null
        return PushEnrolment(
            url,
            prefs.getString(KEY_P256DH, null).orEmpty(),
            prefs.getString(KEY_AUTH, null).orEmpty(),
            kind = "unifiedpush",
        )
    }

    internal fun reset() {
        latest.value = null
    }

    internal suspend fun next(): PushEnrolment? =
        withTimeoutOrNull(ENDPOINT_TIMEOUT_MS) { latest.filterNotNull().first() }

    private fun store(context: Context) = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
}

// The native Android client cannot use Web Push: there is no service worker and no push service
// behind `navigator`. Its transport is UnifiedPush — a distributor app the user already runs,
// speaking the same RFC 8291 encryption the browser does, so the node's sender is unchanged.
private class UnifiedPushPlatform(private val context: Context) : PushPlatform {
    override fun capability(): PushCapability = when {
        UnifiedPush.getDistributors(context).isEmpty() -> PushCapability.NeedsDistributor
        !notificationsAllowed() -> PushCapability.Ready(PushPermission.Denied)
        else -> PushCapability.Ready(PushPermission.Granted)
    }

    // Nothing to hand over: the push service reads the node address and the device token from the
    // same store `Prefs` writes them to, because it runs when no page does.
    override fun prepare(token: String?) = Unit

    override suspend fun subscribe(vapidKey: String): PushEnrolment? = coroutineScope {
        val distributors = UnifiedPush.getDistributors(context)
        if (distributors.isEmpty()) return@coroutineScope null
        // One distributor is the overwhelmingly common case and picking it is not a choice worth
        // a dialog; with several, whatever the user already chose stands.
        if (UnifiedPush.getSavedDistributor(context) == null) {
            UnifiedPush.saveDistributor(context, distributors.first())
        }
        // The collector has to be running before the registration goes out: a distributor on the
        // same device can answer before `register` has returned.
        UnifiedPushEndpoints.reset()
        val arriving = async { UnifiedPushEndpoints.next() }
        UnifiedPush.register(context, UNIFIED_PUSH_INSTANCE, null, vapidKey)
        arriving.await()
    }

    override suspend fun unsubscribe(): String? {
        val endpoint = UnifiedPushEndpoints.saved(context)?.endpoint
        UnifiedPush.unregister(context, UNIFIED_PUSH_INSTANCE)
        UnifiedPushEndpoints.forget(context)
        return endpoint
    }

    override suspend fun enrolment(): PushEnrolment? = UnifiedPushEndpoints.saved(context)

    override fun reconcile(anyBlocked: Boolean, anyDone: Boolean) {
        val manager = context.getSystemService(NotificationManager::class.java) ?: return
        if (!anyBlocked) manager.cancel(BLOCKED_NOTIFICATION_ID)
        if (!anyDone) manager.cancel(DONE_NOTIFICATION_ID)
    }

    private fun notificationsAllowed(): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) ==
            PackageManager.PERMISSION_GRANTED
}

actual fun createPushPlatform(): PushPlatform =
    KamprAndroid.context?.let(::UnifiedPushPlatform) ?: NoPush()
