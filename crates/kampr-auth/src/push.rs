//! Push subscriptions and per-agent delivery rules, in the database that already owns devices.
//!
//! A push subscription is a standing invitation to wake a phone, so it is exactly as sensitive as
//! the device token beside it and belongs under the same revocation. Keeping it here rather than
//! in a second store is what makes "a revoked device's subscriptions die with it" a `JOIN` rather
//! than a cleanup job that can be forgotten.

use crate::secret;
use crate::store::{Store, StoreError};
use serde::{Deserialize, Serialize};
use sqlx::Row;

type Result<T> = std::result::Result<T, StoreError>;

/// How many panes' rules one device may keep, for the same reason `MAX_PANE_PREFS` exists: a
/// device that can name a pane id can otherwise write rows until the disk is full.
pub const MAX_PUSH_RULES: i64 = 256;

/// Every subscription one device may hold. A browser profile and a UnifiedPush distributor are
/// two endpoints on one device, and both are legitimate.
pub const MAX_SUBSCRIPTIONS_PER_DEVICE: i64 = 8;

/// The wildcard pane id: a rule that covers every pane on this device.
pub const ALL_PANES: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PushSubscription {
    pub id: String,
    pub device_id: String,
    /// `webpush` for a browser, `unifiedpush` for a distributor endpoint. Both are RFC 8291
    /// targets and are sent to identically; the label exists so an operator can tell which is
    /// which in the device list.
    pub kind: String,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRule {
    pub pane_id: String,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub snooze_until: Option<i64>,
}

impl PushRule {
    /// Whether this rule silences a notification right now. A snooze that has run out is not a
    /// rule any more, which is why nothing has to sweep the table.
    pub fn silences(&self, now: i64) -> bool {
        self.muted || self.snooze_until.is_some_and(|until| until > now)
    }
}

impl Store {
    /// Upserts on the endpoint rather than on the device: a browser that re-subscribes gets the
    /// same endpoint back, and a stale row under a different device would then double-send.
    pub async fn save_push_subscription(
        &self,
        device_id: &str,
        kind: &str,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
        now: i64,
    ) -> Result<String> {
        let id = hex::encode(secret::random_bytes(8)?);
        sqlx::query(
            "INSERT INTO push_subscriptions (id, device_id, kind, endpoint, p256dh, auth, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(endpoint) DO UPDATE SET
               device_id = excluded.device_id, kind = excluded.kind,
               p256dh = excluded.p256dh, auth = excluded.auth, created_at = excluded.created_at",
        )
        .bind(&id)
        .bind(device_id)
        .bind(kind)
        .bind(endpoint)
        .bind(p256dh)
        .bind(auth)
        .bind(now)
        .execute(self.pool())
        .await?;
        sqlx::query(
            "DELETE FROM push_subscriptions WHERE device_id = ?1 AND endpoint NOT IN
             (SELECT endpoint FROM push_subscriptions WHERE device_id = ?1
              ORDER BY created_at DESC LIMIT ?2)",
        )
        .bind(device_id)
        .bind(MAX_SUBSCRIPTIONS_PER_DEVICE)
        .execute(self.pool())
        .await?;
        Ok(id)
    }

    pub async fn delete_push_subscription(&self, device_id: &str, endpoint: &str) -> Result<bool> {
        let done = sqlx::query("DELETE FROM push_subscriptions WHERE device_id = ? AND endpoint = ?")
            .bind(device_id)
            .bind(endpoint)
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    /// What a push service means by 404 or 410: this endpoint is gone and every further send to
    /// it is noise. Keyed on the endpoint alone because that is all the reply identifies.
    pub async fn forget_push_endpoint(&self, endpoint: &str) -> Result<bool> {
        let done = sqlx::query("DELETE FROM push_subscriptions WHERE endpoint = ?")
            .bind(endpoint)
            .execute(self.pool())
            .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn push_subscriptions_for(&self, device_id: &str) -> Result<Vec<PushSubscription>> {
        let rows = sqlx::query("SELECT * FROM push_subscriptions WHERE device_id = ? ORDER BY created_at")
            .bind(device_id)
            .fetch_all(self.pool())
            .await?;
        Ok(rows.into_iter().map(subscription_from_row).collect())
    }

    /// Every subscription that should be woken for this pane.
    ///
    /// A revoked or expired device has no subscriptions here at all — the join, not a sweep, is
    /// what makes revocation reach the push channel. A pane-specific or wildcard rule that mutes
    /// or is still snoozing removes the device from the set.
    pub async fn push_targets(&self, pane_id: &str, now: i64) -> Result<Vec<PushSubscription>> {
        let rows = sqlx::query(
            "SELECT s.* FROM push_subscriptions s
             JOIN devices d ON d.id = s.device_id
             WHERE d.revoked_at IS NULL AND (d.expires_at IS NULL OR d.expires_at > ?1)
               AND NOT EXISTS (
                 SELECT 1 FROM push_rules r
                 WHERE r.device_id = s.device_id AND r.pane_id IN (?2, ?3)
                   AND (r.muted = 1 OR (r.snooze_until IS NOT NULL AND r.snooze_until > ?1))
               )
             ORDER BY s.created_at",
        )
        .bind(now)
        .bind(pane_id)
        .bind(ALL_PANES)
        .fetch_all(self.pool())
        .await?;
        Ok(rows.into_iter().map(subscription_from_row).collect())
    }

    pub async fn mark_push_sent(&self, id: &str, now: i64) -> Result<()> {
        sqlx::query("UPDATE push_subscriptions SET last_sent_at = ? WHERE id = ?")
            .bind(now)
            .bind(id)
            .execute(self.pool())
            .await?;
        Ok(())
    }

    /// A rule that neither mutes nor snoozes is no rule, so it is deleted rather than stored —
    /// otherwise un-muting every pane once leaves a row per pane forever.
    pub async fn set_push_rule(&self, device_id: &str, rule: &PushRule, now: i64) -> Result<()> {
        if !rule.muted && rule.snooze_until.is_none_or(|until| until <= now) {
            sqlx::query("DELETE FROM push_rules WHERE device_id = ? AND pane_id = ?")
                .bind(device_id)
                .bind(&rule.pane_id)
                .execute(self.pool())
                .await?;
            return Ok(());
        }
        sqlx::query(
            "INSERT INTO push_rules (device_id, pane_id, muted, snooze_until, updated_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(device_id, pane_id) DO UPDATE SET
               muted = excluded.muted, snooze_until = excluded.snooze_until,
               updated_at = excluded.updated_at",
        )
        .bind(device_id)
        .bind(&rule.pane_id)
        .bind(i64::from(rule.muted))
        .bind(rule.snooze_until)
        .bind(now)
        .execute(self.pool())
        .await?;
        sqlx::query(
            "DELETE FROM push_rules WHERE device_id = ?1 AND pane_id NOT IN
             (SELECT pane_id FROM push_rules WHERE device_id = ?1 ORDER BY updated_at DESC LIMIT ?2)",
        )
        .bind(device_id)
        .bind(MAX_PUSH_RULES)
        .execute(self.pool())
        .await?;
        Ok(())
    }

    pub async fn push_rules(&self, device_id: &str) -> Result<Vec<PushRule>> {
        let rows = sqlx::query("SELECT pane_id, muted, snooze_until FROM push_rules WHERE device_id = ?")
            .bind(device_id)
            .fetch_all(self.pool())
            .await?;
        Ok(rows
            .into_iter()
            .map(|r| PushRule {
                pane_id: r.get("pane_id"),
                muted: r.get::<i64, _>("muted") != 0,
                snooze_until: r.get("snooze_until"),
            })
            .collect())
    }
}

fn subscription_from_row(row: sqlx::sqlite::SqliteRow) -> PushSubscription {
    PushSubscription {
        id: row.get("id"),
        device_id: row.get("device_id"),
        kind: row.get("kind"),
        endpoint: row.get("endpoint"),
        p256dh: row.get("p256dh"),
        auth: row.get("auth"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Role;

    const NOW: i64 = 1_700_000_000;

    async fn device(store: &Store, name: &str) -> String {
        store
            .create_device(name, Role::Full, NOW, None, None, None)
            .await
            .unwrap()
            .id
    }

    async fn subscribe(store: &Store, device_id: &str, endpoint: &str) {
        store
            .save_push_subscription(device_id, "webpush", endpoint, "p", "a", NOW)
            .await
            .unwrap();
    }

    /// The whole reason the subscriptions live in this database: revoking a device has to end its
    /// ability to wake a phone, and it must do so without anything else running.
    #[tokio::test]
    async fn a_revoked_device_stops_being_a_push_target() {
        let store = Store::open_memory().await.unwrap();
        let id = device(&store, "phone").await;
        subscribe(&store, &id, "https://push.example/1").await;
        assert_eq!(store.push_targets("n/w1:p1", NOW).await.unwrap().len(), 1);

        store.revoke_device(&id, NOW).await.unwrap();
        assert!(
            store.push_targets("n/w1:p1", NOW).await.unwrap().is_empty(),
            "a revoked device must not be woken"
        );
    }

    #[tokio::test]
    async fn an_expired_device_stops_being_a_push_target() {
        let store = Store::open_memory().await.unwrap();
        let id = store
            .create_device("phone", Role::Full, NOW, Some(NOW + 10), None, None)
            .await
            .unwrap()
            .id;
        subscribe(&store, &id, "https://push.example/1").await;
        assert_eq!(store.push_targets("n/w1:p1", NOW).await.unwrap().len(), 1);
        assert!(store.push_targets("n/w1:p1", NOW + 11).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_mute_is_per_pane_and_a_wildcard_covers_the_lot() {
        let store = Store::open_memory().await.unwrap();
        let id = device(&store, "phone").await;
        subscribe(&store, &id, "https://push.example/1").await;

        store
            .set_push_rule(
                &id,
                &PushRule {
                    pane_id: "n/w1:p1".into(),
                    muted: true,
                    snooze_until: None,
                },
                NOW,
            )
            .await
            .unwrap();
        assert!(store.push_targets("n/w1:p1", NOW).await.unwrap().is_empty());
        assert_eq!(
            store.push_targets("n/w1:p2", NOW).await.unwrap().len(),
            1,
            "muting one agent must not silence the herd"
        );

        store
            .set_push_rule(
                &id,
                &PushRule {
                    pane_id: ALL_PANES.into(),
                    muted: true,
                    snooze_until: None,
                },
                NOW,
            )
            .await
            .unwrap();
        assert!(store.push_targets("n/w1:p2", NOW).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_snooze_expires_by_itself() {
        let store = Store::open_memory().await.unwrap();
        let id = device(&store, "phone").await;
        subscribe(&store, &id, "https://push.example/1").await;
        store
            .set_push_rule(
                &id,
                &PushRule {
                    pane_id: "n/w1:p1".into(),
                    muted: false,
                    snooze_until: Some(NOW + 600),
                },
                NOW,
            )
            .await
            .unwrap();
        assert!(store.push_targets("n/w1:p1", NOW).await.unwrap().is_empty());
        assert_eq!(
            store.push_targets("n/w1:p1", NOW + 601).await.unwrap().len(),
            1,
            "nothing sweeps the table, so the query has to do the expiry"
        );
    }

    /// A browser re-subscribing hands back the same endpoint. Two rows for it would double-send,
    /// and a row still pointing at the old device would send to a device that gave it up.
    #[tokio::test]
    async fn re_subscribing_the_same_endpoint_replaces_rather_than_duplicates() {
        let store = Store::open_memory().await.unwrap();
        let phone = device(&store, "phone").await;
        let laptop = device(&store, "laptop").await;
        subscribe(&store, &phone, "https://push.example/1").await;
        subscribe(&store, &phone, "https://push.example/1").await;
        assert_eq!(store.push_targets("n/w1:p1", NOW).await.unwrap().len(), 1);

        subscribe(&store, &laptop, "https://push.example/1").await;
        let targets = store.push_targets("n/w1:p1", NOW).await.unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].device_id, laptop);
        assert!(store.push_subscriptions_for(&phone).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_gone_endpoint_is_forgotten_and_a_device_cannot_hoard_them() {
        let store = Store::open_memory().await.unwrap();
        let id = device(&store, "phone").await;
        for n in 0..(MAX_SUBSCRIPTIONS_PER_DEVICE + 4) {
            store
                .save_push_subscription(
                    &id,
                    "webpush",
                    &format!("https://push.example/{n}"),
                    "p",
                    "a",
                    NOW + n,
                )
                .await
                .unwrap();
        }
        assert_eq!(
            store.push_subscriptions_for(&id).await.unwrap().len(),
            MAX_SUBSCRIPTIONS_PER_DEVICE as usize
        );

        let live = store.push_subscriptions_for(&id).await.unwrap();
        assert!(store.forget_push_endpoint(&live[0].endpoint).await.unwrap());
        assert_eq!(
            store.push_subscriptions_for(&id).await.unwrap().len(),
            MAX_SUBSCRIPTIONS_PER_DEVICE as usize - 1
        );
    }

    #[tokio::test]
    async fn clearing_a_rule_removes_the_row_rather_than_storing_a_no_op() {
        let store = Store::open_memory().await.unwrap();
        let id = device(&store, "phone").await;
        let muted = PushRule {
            pane_id: "n/w1:p1".into(),
            muted: true,
            snooze_until: None,
        };
        store.set_push_rule(&id, &muted, NOW).await.unwrap();
        assert_eq!(store.push_rules(&id).await.unwrap(), std::slice::from_ref(&muted));

        store
            .set_push_rule(
                &id,
                &PushRule {
                    muted: false,
                    ..muted
                },
                NOW,
            )
            .await
            .unwrap();
        assert!(store.push_rules(&id).await.unwrap().is_empty());
    }
}
