package dev.kampr.shared.net

// Android's authenticator is Credential Manager, not `navigator.credentials`, and it needs an
// Activity and a Digital Asset Links file served from the node's own origin. Until that exists the
// honest answer is no button; see docs/08-threat-model.md §3.2.
actual fun createPasskeys(): Passkeys = NoPasskeys()
