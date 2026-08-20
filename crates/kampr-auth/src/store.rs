use crate::secret;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Full,
    Readonly,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Readonly => "readonly",
        }
    }

    pub fn writes(self) -> bool {
        matches!(self, Self::Full)
    }
}

impl FromStr for Role {
    type Err = ();

    fn from_str(s: &str) -> std::result::Result<Self, ()> {
        match s {
            "full" => Ok(Self::Full),
            "readonly" => Ok(Self::Readonly),
            _ => Err(()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub role: Role,
    pub created_at: i64,
    pub last_seen_at: Option<i64>,
    /// Tier 0 hands out an expiring token on purpose: cleartext access to a machine that can type
    /// into every terminal on it should require a deliberate decision to keep going.
    pub expires_at: Option<i64>,
    pub revoked_at: Option<i64>,
    pub user_agent: Option<String>,
    pub origin: Option<String>,
}

impl Device {
    pub fn active(&self, now: i64) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|e| e > now)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Credential {
    pub id: String,
    pub device_id: String,
    pub rp_id: String,
    pub passkey: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Sql(#[from] sqlx::Error),
    #[error(transparent)]
    Migrate(#[from] sqlx::migrate::MigrateError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Random(#[from] secret::RandomError),
}

type Result<T> = std::result::Result<T, StoreError>;

/// Devices, tokens, passkeys and per-pane preferences.
///
/// This never lives in a plugin root: a GitHub-installed plugin root is a managed checkout that
/// gets replaced wholesale on reinstall, which would silently unpair every device.
#[derive(Debug, Clone)]
pub struct Store {
    pool: SqlitePool,
}

impl Store {
    pub async fn open(path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // `kampr setup` mints a pairing code while the node is serving from the same file, so
        // WAL and a busy timeout are what stop the two processes colliding.
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        restrict(path)?;
        Ok(Self { pool })
    }

    pub async fn open_memory() -> Result<Self> {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(SqliteConnectOptions::from_str("sqlite::memory:").unwrap())
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        Ok(Self { pool })
    }

    pub async fn create_device(
        &self,
        name: &str,
        role: Role,
        now: i64,
        expires_at: Option<i64>,
        user_agent: Option<&str>,
        origin: Option<&str>,
    ) -> Result<Device> {
        let id = hex::encode(secret::random_bytes(8)?);
        sqlx::query(
            "INSERT INTO devices (id, name, role, created_at, expires_at, user_agent, origin)
             VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(name)
        .bind(role.as_str())
        .bind(now)
        .bind(expires_at)
        .bind(user_agent)
        .bind(origin)
        .execute(&self.pool)
        .await?;
        Ok(Device {
            id,
            name: name.to_string(),
            role,
            created_at: now,
            last_seen_at: None,
            expires_at,
            revoked_at: None,
            user_agent: user_agent.map(str::to_string),
            origin: origin.map(str::to_string),
        })
    }

    pub async fn mint_token(&self, device_id: &str, now: i64, expires_at: Option<i64>) -> Result<String> {
        let token = secret::token()?;
        sqlx::query("INSERT INTO tokens (hash, device_id, created_at, expires_at) VALUES (?, ?, ?, ?)")
            .bind(secret::digest(&token))
            .bind(device_id)
            .bind(now)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        Ok(token)
    }

    /// Resolves a bearer token to the device it is bound to. A token whose device is revoked or
    /// expired resolves to nothing, so revoking a device is enough — the tokens need no sweep.
    pub async fn device_for_token(&self, token: &str, now: i64) -> Result<Option<Device>> {
        let row = sqlx::query(
            "SELECT d.* FROM tokens t JOIN devices d ON d.id = t.device_id
             WHERE t.hash = ? AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > ?)",
        )
        .bind(secret::digest(token))
        .bind(now)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(device_from_row).filter(|d| d.active(now)))
    }

    pub async fn touch_device(&self, device_id: &str, now: i64) -> Result<()> {
        sqlx::query("UPDATE devices SET last_seen_at = ? WHERE id = ?")
            .bind(now)
            .bind(device_id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn devices(&self) -> Result<Vec<Device>> {
        let rows = sqlx::query("SELECT * FROM devices ORDER BY created_at")
            .fetch_all(&self.pool)
            .await?;
        Ok(rows.into_iter().map(device_from_row).collect())
    }

    pub async fn device(&self, id: &str) -> Result<Option<Device>> {
        let row = sqlx::query("SELECT * FROM devices WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(device_from_row))
    }

    pub async fn revoke_device(&self, id: &str, now: i64) -> Result<bool> {
        let done = sqlx::query("UPDATE devices SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn set_role(&self, id: &str, role: Role) -> Result<bool> {
        let done = sqlx::query("UPDATE devices SET role = ? WHERE id = ?")
            .bind(role.as_str())
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn extend_device(&self, id: &str, expires_at: Option<i64>) -> Result<bool> {
        let done = sqlx::query("UPDATE devices SET expires_at = ? WHERE id = ?")
            .bind(expires_at)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(done.rows_affected() > 0)
    }

    pub async fn create_pairing(&self, role: Role, now: i64, expires_at: i64) -> Result<String> {
        let code = secret::pairing_code()?;
        sqlx::query("INSERT INTO pairings (hash, role, created_at, expires_at) VALUES (?, ?, ?, ?)")
            .bind(secret::digest(&secret::normalise_code(&code)))
            .bind(role.as_str())
            .bind(now)
            .bind(expires_at)
            .execute(&self.pool)
            .await?;
        Ok(code)
    }

    /// Single use: the row is marked spent in the same statement that claims it, so two devices
    /// racing on one code cannot both win.
    pub async fn claim_pairing(&self, code: &str, now: i64) -> Result<Option<Role>> {
        let hash = secret::digest(&secret::normalise_code(code));
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT role FROM pairings
             WHERE hash = ? AND used_at IS NULL AND expires_at > ? AND attempts < 10",
        )
        .bind(&hash)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        sqlx::query("UPDATE pairings SET used_at = ? WHERE hash = ?")
            .bind(now)
            .bind(&hash)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(row.get::<String, _>("role").parse().ok())
    }

    pub async fn pending_pairings(&self, now: i64) -> Result<u32> {
        let row = sqlx::query("SELECT COUNT(*) AS n FROM pairings WHERE used_at IS NULL AND expires_at > ?")
            .bind(now)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.get::<i64, _>("n") as u32)
    }

    pub async fn expire_pairings(&self, now: i64) -> Result<()> {
        sqlx::query("DELETE FROM pairings WHERE expires_at <= ? OR used_at IS NOT NULL")
            .bind(now)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn save_credential(&self, cred: &Credential, now: i64) -> Result<()> {
        sqlx::query(
            "INSERT INTO credentials (id, device_id, rp_id, passkey, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET passkey = excluded.passkey, last_used_at = excluded.created_at",
        )
        .bind(&cred.id)
        .bind(&cred.device_id)
        .bind(&cred.rp_id)
        .bind(&cred.passkey)
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Only credentials for live devices at this RP ID: a passkey registered against a hostname
    /// the node no longer answers on must not authenticate anyone.
    pub async fn credentials(&self, rp_id: &str, now: i64) -> Result<Vec<Credential>> {
        let rows = sqlx::query(
            "SELECT c.* FROM credentials c JOIN devices d ON d.id = c.device_id
             WHERE c.rp_id = ? AND d.revoked_at IS NULL AND (d.expires_at IS NULL OR d.expires_at > ?)",
        )
        .bind(rp_id)
        .bind(now)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| Credential {
                id: r.get("id"),
                device_id: r.get("device_id"),
                rp_id: r.get("rp_id"),
                passkey: r.get("passkey"),
            })
            .collect())
    }

    pub async fn credential(&self, id: &str) -> Result<Option<Credential>> {
        let row = sqlx::query("SELECT * FROM credentials WHERE id = ?")
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| Credential {
            id: r.get("id"),
            device_id: r.get("device_id"),
            rp_id: r.get("rp_id"),
            passkey: r.get("passkey"),
        }))
    }

    pub async fn touch_credential(&self, id: &str, passkey: &str, now: i64) -> Result<()> {
        sqlx::query("UPDATE credentials SET passkey = ?, last_used_at = ? WHERE id = ?")
            .bind(passkey)
            .bind(now)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn set_pane_prefs(
        &self,
        device_id: &str,
        pane_id: &str,
        prefs: &serde_json::Value,
        now: i64,
    ) -> Result<()> {
        sqlx::query(
            "INSERT INTO pane_prefs (device_id, pane_id, prefs, updated_at) VALUES (?, ?, ?, ?)
             ON CONFLICT(device_id, pane_id) DO UPDATE
             SET prefs = excluded.prefs, updated_at = excluded.updated_at",
        )
        .bind(device_id)
        .bind(pane_id)
        .bind(prefs.to_string())
        .bind(now)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn pane_prefs(&self, device_id: &str) -> Result<serde_json::Value> {
        let rows = sqlx::query("SELECT pane_id, prefs FROM pane_prefs WHERE device_id = ?")
            .bind(device_id)
            .fetch_all(&self.pool)
            .await?;
        let map = rows
            .into_iter()
            .map(|r| {
                let prefs: String = r.get("prefs");
                (
                    r.get::<String, _>("pane_id"),
                    serde_json::from_str(&prefs).unwrap_or(serde_json::Value::Null),
                )
            })
            .collect::<serde_json::Map<_, _>>();
        Ok(serde_json::Value::Object(map))
    }
}

fn device_from_row(row: sqlx::sqlite::SqliteRow) -> Device {
    Device {
        id: row.get("id"),
        name: row.get("name"),
        role: row.get::<String, _>("role").parse().unwrap_or_default(),
        created_at: row.get("created_at"),
        last_seen_at: row.get("last_seen_at"),
        expires_at: row.get("expires_at"),
        revoked_at: row.get("revoked_at"),
        user_agent: row.get("user_agent"),
        origin: row.get("origin"),
    }
}

#[cfg(unix)]
fn restrict(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    async fn store() -> Store {
        Store::open_memory().await.unwrap()
    }

    #[tokio::test]
    async fn a_token_resolves_to_the_device_it_was_minted_for() {
        let s = store().await;
        let d = s
            .create_device("phone", Role::Full, NOW, None, None, None)
            .await
            .unwrap();
        let token = s.mint_token(&d.id, NOW, None).await.unwrap();
        assert_eq!(s.device_for_token(&token, NOW).await.unwrap().unwrap().id, d.id);
        assert_eq!(s.device_for_token("kmp_nope", NOW).await.unwrap(), None);
    }

    #[tokio::test]
    async fn revoking_a_device_kills_its_token_without_touching_the_token_row() {
        let s = store().await;
        let d = s
            .create_device("phone", Role::Full, NOW, None, None, None)
            .await
            .unwrap();
        let token = s.mint_token(&d.id, NOW, None).await.unwrap();
        assert!(s.revoke_device(&d.id, NOW + 1).await.unwrap());
        assert_eq!(s.device_for_token(&token, NOW + 2).await.unwrap(), None);
        assert!(!s.revoke_device(&d.id, NOW + 3).await.unwrap());
    }

    #[tokio::test]
    async fn an_expired_device_stops_authenticating() {
        let s = store().await;
        let d = s
            .create_device("phone", Role::Full, NOW, Some(NOW + 10), None, None)
            .await
            .unwrap();
        let token = s.mint_token(&d.id, NOW, Some(NOW + 10)).await.unwrap();
        assert!(s.device_for_token(&token, NOW + 5).await.unwrap().is_some());
        assert_eq!(s.device_for_token(&token, NOW + 11).await.unwrap(), None);
        assert!(s.extend_device(&d.id, Some(NOW + 100)).await.unwrap());
        assert_eq!(
            s.device_for_token(&token, NOW + 11).await.unwrap(),
            None,
            "the token expired too"
        );
    }

    #[tokio::test]
    async fn a_pairing_code_is_single_use_and_time_boxed() {
        let s = store().await;
        let code = s.create_pairing(Role::Readonly, NOW, NOW + 600).await.unwrap();
        assert_eq!(s.claim_pairing(&code, NOW).await.unwrap(), Some(Role::Readonly));
        assert_eq!(s.claim_pairing(&code, NOW).await.unwrap(), None);

        let stale = s.create_pairing(Role::Full, NOW, NOW + 600).await.unwrap();
        assert_eq!(s.claim_pairing(&stale, NOW + 601).await.unwrap(), None);
    }

    #[tokio::test]
    async fn a_pairing_code_matches_however_it_was_typed() {
        let s = store().await;
        let code = s.create_pairing(Role::Full, NOW, NOW + 600).await.unwrap();
        let typed = code.replace('-', " ").to_lowercase();
        assert_eq!(s.claim_pairing(&typed, NOW).await.unwrap(), Some(Role::Full));
    }

    #[tokio::test]
    async fn credentials_are_scoped_to_the_rp_id_and_to_live_devices() {
        let s = store().await;
        let d = s
            .create_device("phone", Role::Full, NOW, None, None, None)
            .await
            .unwrap();
        let cred = Credential {
            id: "cred1".into(),
            device_id: d.id.clone(),
            rp_id: "kampr.example.com".into(),
            passkey: "{}".into(),
        };
        s.save_credential(&cred, NOW).await.unwrap();
        assert_eq!(s.credentials("kampr.example.com", NOW).await.unwrap().len(), 1);
        assert_eq!(
            s.credentials("elsewhere.example.com", NOW).await.unwrap().len(),
            0
        );
        s.revoke_device(&d.id, NOW).await.unwrap();
        assert_eq!(s.credentials("kampr.example.com", NOW).await.unwrap().len(), 0);
    }

    #[tokio::test]
    async fn pane_prefs_are_per_device() {
        let s = store().await;
        let a = s
            .create_device("a", Role::Full, NOW, None, None, None)
            .await
            .unwrap();
        let b = s
            .create_device("b", Role::Full, NOW, None, None, None)
            .await
            .unwrap();
        s.set_pane_prefs(&a.id, "n/w1:p1", &serde_json::json!({"zoom": 1.5}), NOW)
            .await
            .unwrap();
        assert_eq!(s.pane_prefs(&a.id).await.unwrap()["n/w1:p1"]["zoom"], 1.5);
        assert_eq!(s.pane_prefs(&b.id).await.unwrap(), serde_json::json!({}));
    }
}
