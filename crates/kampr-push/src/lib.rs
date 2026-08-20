//! Web Push for a Kampr node: VAPID identity, the notification a blocked agent produces, the
//! batching that keeps three of them from racing, and the delivery itself.
//!
//! **Subscriptions live in `kampr-auth`'s database, not here.** A push subscription is a standing
//! invitation to wake a phone and is exactly as sensitive as the device token beside it, so it
//! belongs under the same revocation — `kampr_auth::Store::push_targets` is a join against live
//! devices rather than a cleanup job that can be forgotten.
//!
//! **A UnifiedPush endpoint is a Web Push endpoint.** UnifiedPush 3.0 carries RFC 8291 encryption
//! and VAPID, so a distributor's endpoint is delivered to by exactly this code — the only thing
//! that differs is who hands the endpoint over, which is why `kind` is a label and never a branch.

pub mod batch;
pub mod note;
pub mod send;
pub mod vapid;

pub use batch::{Batch, WINDOW, collect, per_target};
pub use note::{Blocked, Notification, TAG};
pub use send::{Outcome, Sender};
pub use vapid::{Vapid, VapidError, subject_for};
