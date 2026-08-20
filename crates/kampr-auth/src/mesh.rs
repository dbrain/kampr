use crate::secret;
use crate::store::{Store, StoreError};
use serde::Serialize;
use sqlx::Row;

type Result<T> = std::result::Result<T, StoreError>;

/// Wrong guesses against outstanding join codes before every outstanding code is dead. Same
/// bound, and the same reasoning, as [`crate::store::PAIRING_ATTEMPT_LIMIT`].
pub const INVITE_ATTEMPT_LIMIT: i64 = 10;

/// Which way the link is dialled, which is the only asymmetry between two nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MeshRole {
    /// It dials us. We hold its key so we can recognise it when it does.
    Peer,
    /// We dial it. We hold its key so we can refuse an impostor answering at that URL.
    Hub,
}

impl MeshRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Peer => "peer",
            Self::Hub => "hub",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MeshNode {
    /// Hex ed25519 verifying key. The credential, and the primary key: a node that regenerates
    /// its identity is a different node and has to be enrolled again.
    pub pubkey: String,
    pub node_id: String,
    pub name: String,
    pub role: MeshRole,
    pub url: Option<String>,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    pub revoked_at: Option<i64>,
}

impl MeshNode {
    pub fn active(&self) -> bool {
        self.revoked_at.is_none()
    }

    pub fn fingerprint(&self) -> String {
        crate::identity::fingerprint_of(&self.pubkey)
    }
}

/// Enrolled nodes and the join codes that enrol them.
///
/// Deliberately a different table and a different credential from `devices`: a device token
/// authenticates a person's browser, an ed25519 key authenticates a node, and neither can be
/// presented in the other's place. A compromised viewer session holds a bearer token, which the
/// mesh handshake never asks for.
pub struct Mesh<'a> {
    store: &'a Store,
}

impl Store {
    pub fn mesh(&self) -> Mesh<'_> {
        Mesh { store: self }
    }
}

impl Mesh<'_> {
    /// A single-use join code. Short-lived and rate limited, exactly like a pairing code — it
    /// authorises one enrolment and then the ed25519 key is the credential forever after.
    pub async fn invite(&self, now: i64, expires_at: i64) -> Result<String> {
        let code = secret::pairing_code()?;
        sqlx::query("INSERT INTO mesh_invites (hash, created_at, expires_at) VALUES (?, ?, ?)")
            .bind(secret::pairing_digest(&secret::normalise_code(&code)))
            .bind(now)
            .bind(expires_at)
            .execute(self.store.pool())
            .await?;
        Ok(code)
    }

    /// Marks the code spent in the same statement that claims it, so two nodes racing on one code
    /// cannot both win. A miss charges every outstanding code an attempt.
    pub async fn claim_invite(&self, code: &str, now: i64) -> Result<bool> {
        let hash = secret::pairing_digest(&secret::normalise_code(code));
        let mut tx = self.store.pool().begin().await?;
        let row = sqlx::query(
            "SELECT hash FROM mesh_invites
             WHERE hash = ? AND used_at IS NULL AND expires_at > ? AND attempts < ?",
        )
        .bind(&hash)
        .bind(now)
        .bind(INVITE_ATTEMPT_LIMIT)
        .fetch_optional(&mut *tx)
        .await?;
        if row.is_none() {
            sqlx::query(
                "UPDATE mesh_invites SET attempts = attempts + 1
                 WHERE used_at IS NULL AND expires_at > ?",
            )
            .bind(now)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(false);
        }
        sqlx::query("UPDATE mesh_invites SET used_at = ? WHERE hash = ?")
            .bind(now)
            .bind(&hash)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(true)
    }

    pub async fn pending_invites(&self, now: i64) -> Result<u32> {
        let row =
            sqlx::query("SELECT COUNT(*) AS n FROM mesh_invites WHERE used_at IS NULL AND expires_at > ?")
                .bind(now)
                .fetch_one(self.store.pool())
                .await?;
        Ok(row.get::<i64, _>("n") as u32)
    }

    /// Re-enrolling a key that is already known keeps its `created_at` and clears a revocation —
    /// which is what an operator asks for by handing out a fresh join code for it.
    pub async fn enrol(
        &self,
        pubkey: &str,
        node_id: &str,
        name: &str,
        role: MeshRole,
        url: Option<&str>,
        now: i64,
    ) -> Result<MeshNode> {
        sqlx::query(
            "INSERT INTO mesh_nodes (pubkey, node_id, name, role, url, created_at)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(pubkey) DO UPDATE SET
               node_id = excluded.node_id, name = excluded.name, role = excluded.role,
               url = COALESCE(excluded.url, mesh_nodes.url), revoked_at = NULL",
        )
        .bind(pubkey)
        .bind(node_id)
        .bind(name)
        .bind(role.as_str())
        .bind(url)
        .bind(now)
        .execute(self.store.pool())
        .await?;
        Ok(self.node(pubkey).await?.expect("the row just written"))
    }

    pub async fn node(&self, pubkey: &str) -> Result<Option<MeshNode>> {
        let row = sqlx::query("SELECT * FROM mesh_nodes WHERE pubkey = ?")
            .bind(pubkey)
            .fetch_optional(self.store.pool())
            .await?;
        Ok(row.map(node_from_row))
    }

    pub async fn nodes(&self, role: MeshRole) -> Result<Vec<MeshNode>> {
        let rows = sqlx::query("SELECT * FROM mesh_nodes WHERE role = ? ORDER BY created_at")
            .bind(role.as_str())
            .fetch_all(self.store.pool())
            .await?;
        Ok(rows.into_iter().map(node_from_row).collect())
    }

    /// Accepts a public key, a node id, a fingerprint or a name, because those are the four
    /// things an operator has in front of them when they decide to cut a node off.
    pub async fn revoke(&self, needle: &str, now: i64) -> Result<Option<MeshNode>> {
        let Some(node) = self.find(needle).await? else {
            return Ok(None);
        };
        sqlx::query("UPDATE mesh_nodes SET revoked_at = ? WHERE pubkey = ? AND revoked_at IS NULL")
            .bind(now)
            .bind(&node.pubkey)
            .execute(self.store.pool())
            .await?;
        self.node(&node.pubkey).await
    }

    pub async fn forget(&self, needle: &str) -> Result<Option<MeshNode>> {
        let Some(node) = self.find(needle).await? else {
            return Ok(None);
        };
        sqlx::query("DELETE FROM mesh_nodes WHERE pubkey = ?")
            .bind(&node.pubkey)
            .execute(self.store.pool())
            .await?;
        Ok(Some(node))
    }

    pub async fn find(&self, needle: &str) -> Result<Option<MeshNode>> {
        let rows = sqlx::query("SELECT * FROM mesh_nodes")
            .fetch_all(self.store.pool())
            .await?;
        let needle = needle.trim();
        Ok(rows.into_iter().map(node_from_row).find(|n| {
            n.pubkey == needle || n.node_id == needle || n.name == needle || n.fingerprint() == needle
        }))
    }

    pub async fn touch(&self, pubkey: &str, now: i64) -> Result<()> {
        sqlx::query("UPDATE mesh_nodes SET last_seen_at = ? WHERE pubkey = ?")
            .bind(now)
            .bind(pubkey)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }

    /// The authorization row a hub gets on the peer it dials.
    ///
    /// Authentication is the ed25519 handshake and nothing else; this row is what the *session*
    /// layer already checks before every write, so reusing it means a hub is listed alongside the
    /// phones and is cut off the same way. No token is ever minted against it, so a stolen bearer
    /// can never present itself as one — and a revocation here sticks, because the mesh handshake
    /// does not clear it.
    pub async fn hub_device(&self, pubkey: &str, name: &str, now: i64) -> Result<crate::Device> {
        let id = format!("mesh:{}", &pubkey[..pubkey.len().min(16)]);
        sqlx::query(
            "INSERT INTO devices (id, name, role, created_at) VALUES (?, ?, 'full', ?)
             ON CONFLICT(id) DO UPDATE SET name = excluded.name",
        )
        .bind(&id)
        .bind(name)
        .bind(now)
        .execute(self.store.pool())
        .await?;
        self.store
            .device(&id)
            .await?
            .ok_or_else(|| StoreError::Sql(sqlx::Error::RowNotFound))
    }

    pub async fn expire_invites(&self, now: i64) -> Result<()> {
        sqlx::query("DELETE FROM mesh_invites WHERE expires_at <= ? OR used_at IS NOT NULL")
            .bind(now)
            .execute(self.store.pool())
            .await?;
        Ok(())
    }
}

fn node_from_row(row: sqlx::sqlite::SqliteRow) -> MeshNode {
    MeshNode {
        pubkey: row.get("pubkey"),
        node_id: row.get("node_id"),
        name: row.get("name"),
        role: match row.get::<String, _>("role").as_str() {
            "hub" => MeshRole::Hub,
            _ => MeshRole::Peer,
        },
        url: row.get("url"),
        created_at: row.get("created_at"),
        last_seen_at: row.get("last_seen_at"),
        revoked_at: row.get("revoked_at"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    async fn store() -> Store {
        Store::open_memory().await.unwrap()
    }

    #[tokio::test]
    async fn a_join_code_enrols_once_and_is_then_spent() {
        let store = store().await;
        let mesh = store.mesh();
        let code = mesh.invite(100, 700).await.unwrap();
        assert!(mesh.claim_invite(&code, 200).await.unwrap());
        assert!(
            !mesh.claim_invite(&code, 200).await.unwrap(),
            "a join code enrols one node, not a queue of them"
        );
    }

    #[tokio::test]
    async fn an_expired_or_unknown_code_never_claims() {
        let store = store().await;
        let mesh = store.mesh();
        let code = mesh.invite(100, 700).await.unwrap();
        assert!(!mesh.claim_invite(&code, 800).await.unwrap());
        assert!(!mesh.claim_invite("ZZZZ-ZZZZ", 200).await.unwrap());
    }

    #[tokio::test]
    async fn guessing_burns_the_outstanding_codes() {
        let store = store().await;
        let mesh = store.mesh();
        let code = mesh.invite(100, 700).await.unwrap();
        for _ in 0..INVITE_ATTEMPT_LIMIT {
            assert!(!mesh.claim_invite("ZZZZ-ZZZZ", 200).await.unwrap());
        }
        assert!(
            !mesh.claim_invite(&code, 200).await.unwrap(),
            "the guessing budget is spent, so the code is dead rather than guessable"
        );
    }

    #[tokio::test]
    async fn an_enrolled_key_is_found_by_every_name_an_operator_has_for_it() {
        let store = store().await;
        let mesh = store.mesh();
        let node = mesh
            .enrol(KEY, "01JNODE", "laptop", MeshRole::Peer, None, 100)
            .await
            .unwrap();
        for needle in [KEY, "01JNODE", "laptop", &node.fingerprint()] {
            assert_eq!(mesh.find(needle).await.unwrap().unwrap().pubkey, KEY);
        }
        assert!(mesh.find(OTHER).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn revoking_leaves_the_row_but_kills_the_credential() {
        let store = store().await;
        let mesh = store.mesh();
        mesh.enrol(KEY, "01JNODE", "laptop", MeshRole::Peer, None, 100)
            .await
            .unwrap();
        assert!(mesh.node(KEY).await.unwrap().unwrap().active());
        let revoked = mesh.revoke("laptop", 200).await.unwrap().unwrap();
        assert!(!revoked.active());
        assert_eq!(revoked.revoked_at, Some(200));
        assert!(mesh.revoke("nobody", 200).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn re_enrolling_a_revoked_key_brings_it_back_without_a_second_row() {
        let store = store().await;
        let mesh = store.mesh();
        mesh.enrol(KEY, "01JNODE", "laptop", MeshRole::Peer, None, 100)
            .await
            .unwrap();
        mesh.revoke(KEY, 200).await.unwrap();
        let back = mesh
            .enrol(KEY, "01JNODE", "laptop", MeshRole::Peer, None, 300)
            .await
            .unwrap();
        assert!(back.active());
        assert_eq!(back.created_at, 100, "the same node, not a new one");
        assert_eq!(mesh.nodes(MeshRole::Peer).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_hub_gets_one_device_row_that_no_token_can_ever_present() {
        let store = store().await;
        let mesh = store.mesh();
        let first = mesh.hub_device(KEY, "hub front", 100).await.unwrap();
        let again = mesh.hub_device(KEY, "hub front", 200).await.unwrap();
        assert_eq!(first.id, again.id, "reconnecting reuses the row");
        assert!(first.id.starts_with("mesh:"));
        assert!(first.role.writes());
        assert!(
            store.device_for_token(&first.id, 300).await.unwrap().is_none(),
            "there is no token to present"
        );
    }

    #[tokio::test]
    async fn revoking_a_hubs_device_survives_it_reconnecting() {
        let store = store().await;
        let mesh = store.mesh();
        let device = mesh.hub_device(KEY, "hub front", 100).await.unwrap();
        store.revoke_device(&device.id, 200).await.unwrap();
        let back = mesh.hub_device(KEY, "hub front", 300).await.unwrap();
        assert!(!back.active(400), "the mesh handshake does not undo a revocation");
    }

    #[tokio::test]
    async fn hubs_and_peers_are_listed_apart() {
        let store = store().await;
        let mesh = store.mesh();
        mesh.enrol(KEY, "01JA", "laptop", MeshRole::Peer, None, 100)
            .await
            .unwrap();
        mesh.enrol(
            OTHER,
            "01JB",
            "front",
            MeshRole::Hub,
            Some("wss://kampr.example.com"),
            100,
        )
        .await
        .unwrap();
        assert_eq!(mesh.nodes(MeshRole::Peer).await.unwrap().len(), 1);
        let hubs = mesh.nodes(MeshRole::Hub).await.unwrap();
        assert_eq!(hubs[0].url.as_deref(), Some("wss://kampr.example.com"));
    }
}
