package dev.kampr.shared.push

// Must match `kampr_push::note::TAG` and the service worker's, so the newest notification replaces
// the last rather than stacking a column of stale prompts on a phone that was away.
//
// One id is also why the payload is the whole outstanding set rather than the pane that just
// changed: whatever arrives last is the only thing in the shade, so a payload naming less than
// everything silently unsays the rest.
//
// Here rather than beside the builder in `androidApp` because two things address this one
// notification from opposite directions — the push service posts it, and the live herd takes it
// down — and only this module is visible to both.
const val BLOCKED_CHANNEL = "kampr.blocked"
const val BLOCKED_NOTIFICATION_ID = 1
