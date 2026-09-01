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

// One tag per notification kind, matching `kampr_push::note::Kind::tag` and the service worker's.
// Two tags is two slots in the shade: whatever arrives last under a tag is the only thing showing
// for that kind, so a finished agent sharing the blocked tag would take a live question off the
// phone.
const val TAG_BLOCKED = "kampr.blocked"
const val TAG_DONE = "kampr.done"

// The payload version this client can read, sent with every subscription.
//
// It is what stops the node delivering a notification kind this build would render as a different
// one: a client with a single notification slot posts whatever arrives into it, so a node that
// sent it a `done` would take a live question off the phone. Raise it only alongside the code
// that actually handles the new kind. Must match `kampr_push::note::VERSION`.
const val PUSH_PAYLOAD_VERSION = 3

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

    // The subscription this device already holds, read without prompting for anything.
    //
    // It exists so a client can re-announce itself: what the node records against a subscription
    // includes `PUSH_PAYLOAD_VERSION`, and a device that subscribed under an older build has an
    // older number on file. Nothing on screen would say so — the notifications it can now handle
    // would simply never arrive, which is the shape of #233 — so the announcement is made on
    // connect rather than waiting for somebody to open the notifications screen.
    suspend fun enrolment(): PushEnrolment?

    // A notification is a summary of the moment the node sent it. When this client can see the
    // herd for itself the herd is fresher, so a prompt answered anywhere — at the desk, in the
    // TUI, on another phone — comes down without waiting for a push to say so.
    //
    // Two slots, reconciled independently, because they empty for different reasons: a question
    // is answered anywhere in the herd, and a finish is *read* — which is a fact this device
    // holds alone (`SeenDone`) and never tells the node, since clearing herdr's own marker would
    // take a focus op and that is the operator's press (rule 3).
    //
    // It only ever takes one down. Re-posting a shrunken summary would mean reproducing the
    // node's own title and body shaping in every client, and the resync push already does that
    // from the one place that holds the questions.
    fun reconcile(anyBlocked: Boolean, anyDone: Boolean)
}

expect fun createPushPlatform(): PushPlatform

// The shape a target with no push channel at all takes: the desktop JVM build, which is already on
// the screen the herd is running on.
class NoPush(private val why: PushCapability = PushCapability.Unsupported) : PushPlatform {
    override fun capability(): PushCapability = why
    override fun prepare(token: String?) = Unit
    override suspend fun subscribe(vapidKey: String): PushEnrolment? = null
    override suspend fun unsubscribe(): String? = null
    override suspend fun enrolment(): PushEnrolment? = null
    override fun reconcile(anyBlocked: Boolean, anyDone: Boolean) = Unit
}
