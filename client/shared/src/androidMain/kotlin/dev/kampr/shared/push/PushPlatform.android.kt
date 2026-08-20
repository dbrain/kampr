package dev.kampr.shared.push

// The native Android client cannot use Web Push: there is no service worker and no push service
// behind `navigator`. Its transport is UnifiedPush — a distributor app the user already runs,
// speaking the same RFC 8291 encryption the browser does, so the node's sender is unchanged.
// Registration is the app's job, not this interface's; see docs/08-notifications.md.
actual fun createPushPlatform(): PushPlatform = NoPush()
