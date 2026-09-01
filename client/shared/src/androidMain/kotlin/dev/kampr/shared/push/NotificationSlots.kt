package dev.kampr.shared.push

// The two notification slots, one per kind the node sends. Each channel and id must match a
// `kampr_push::note::Kind`'s tag, so the newest notification of a kind replaces the last of that
// kind rather than stacking a column of stale ones on a phone that was away.
//
// **Two slots and not one.** One id is one notification: whatever arrives last is the only thing
// in the shade, so a finished agent sharing the blocked slot would take a live question off the
// phone. They also differ in what they are worth interrupting for, and an Android channel's
// importance is fixed per channel — sharing one means either a question that does not buzz or a
// finish that does.
//
// One id *per kind* is also why each payload is that kind's whole outstanding set rather than the
// pane that just changed: a payload naming less than everything silently unsays the rest.
//
// Here rather than beside the builders in `androidApp` because two things address these
// notifications from opposite directions — the push service posts them, and the live herd takes
// them down — and only this module is visible to both.
const val BLOCKED_CHANNEL = TAG_BLOCKED
const val BLOCKED_NOTIFICATION_ID = 1

const val DONE_CHANNEL = TAG_DONE
const val DONE_NOTIFICATION_ID = 2
