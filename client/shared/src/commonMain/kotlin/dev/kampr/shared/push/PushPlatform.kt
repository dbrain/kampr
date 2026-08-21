package dev.kampr.shared.push

// What this platform can do about notifications, decided by the platform and never guessed from
// the URL. The client renders exactly one of these; there is no fallback that "tries anyway".
sealed interface PushCapability {
    // No push API at all: the desktop JVM build, or a browser too old for one.
    data object Unsupported : PushCapability

    // iOS grants Web Push ONLY to a Home Screen web app, since 16.4. A Safari tab can neither
    // subscribe nor be told why, so the honest surface is an Add to Home Screen prompt rather
    // than a button that fails inside the permission call (findings §3.7).
    data object NeedsHomeScreen : PushCapability

    // The origin is not a secure context. Nothing a client does can change that; a hostname can.
    data object InsecureContext : PushCapability

    // Android has no push service of its own that does not go through Google. UnifiedPush's
    // distributor is a separate app the user installs and points wherever they like, and until
    // one is there there is nothing to register with — which is a thing to say, not to swallow.
    data object NeedsDistributor : PushCapability

    data class Ready(val permission: PushPermission) : PushCapability
}

enum class PushPermission { Default, Granted, Denied }

// A browser's PushSubscription, in the shape `/api/push/subscribe` takes.
data class PushEnrolment(
    val endpoint: String,
    val p256dh: String,
    val auth: String,
    val kind: String = "webpush",
)

interface PushPlatform {
    fun capability(): PushCapability

    // Registers the service worker and hands it the device token, so it can warm the cache behind
    // a push while no page is open. Cheap and idempotent.
    fun prepare(token: String?)

    // **Must be called from a user gesture.** Browsers refuse `Notification.requestPermission`
    // outside one, and refuse it silently enough that a retry loop looks like a bug in the node.
    suspend fun subscribe(vapidKey: String): PushEnrolment?

    // The endpoint that was dropped, so the node can forget the row it belongs to.
    suspend fun unsubscribe(): String?

    suspend fun currentEndpoint(): String?
}

expect fun createPushPlatform(): PushPlatform

// The shape a target with no push channel at all takes: the desktop JVM build, which is already on
// the screen the herd is running on.
class NoPush(private val why: PushCapability = PushCapability.Unsupported) : PushPlatform {
    override fun capability(): PushCapability = why
    override fun prepare(token: String?) = Unit
    override suspend fun subscribe(vapidKey: String): PushEnrolment? = null
    override suspend fun unsubscribe(): String? = null
    override suspend fun currentEndpoint(): String? = null
}
