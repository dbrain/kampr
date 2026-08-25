use crate::files::{chmod, private_dir, touch_private};
use crate::secret;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::{Row, SqlitePool};
use std::path::Path;
use std::str::FromStr;

/// `Readonly` is the default so an unreadable role fails closed. A row whose `role` column has been
/// corrupted, or written by a newer version that knows a role this build does not, must not read
/// back as a device that can type into every terminal on the host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Full,
    #[default]
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

/// What one [`Store::extend_device`] actually did, so an audit line can say it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Extension {
    pub found: bool,
    pub tokens: u64,
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

/// Wrong guesses against outstanding codes, after which every outstanding code is dead. The
/// per-peer limiter slows one attacker down; this is what bounds the total number of guesses a
/// ~40-bit code ever has to survive, whatever the peer address says.
pub const PAIRING_ATTEMPT_LIMIT: i64 = 10;

/// Per-pane preferences a device may keep. Nothing prunes rows for panes that no longer exist,
/// and a device that can name a pane id can otherwise write rows until the disk is full.
pub const MAX_PANE_PREFS: i64 = 256;

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
            private_dir(dir)?;
        }
        // sqlite creates `-wal` and `-shm` itself at the process umask, and the write-ahead log
        // holds a pairing digest long before it is checkpointed into the main file. Claiming the
        // main file before the pool opens and tightening the sidecars after is what keeps all
        // three off a local unprivileged reader.
        touch_private(path)?;
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
            .busy_timeout(std::time::Duration::from_secs(5));
        let db = path.to_path_buf();
        let pool = SqlitePoolOptions::new()
            .max_connections(4)
            // sqlite unlinks `-wal` and `-shm` when the last connection closes and recreates them
            // at the process umask when the next one opens, so an idle node's credential digests
            // come back 0644 on a default host. Keeping one connection for the life of the
            // process means the sidecars `restrict` tightened are the ones that stay.
            .min_connections(1)
            .idle_timeout(None)
            .max_lifetime(None)
            // And for every connection past that one, because a sidecar sqlite recreated is
            // recreated by whichever connection opened it.
            .after_connect(move |_, _| {
                let db = db.clone();
                Box::pin(async move { restrict(&db).map_err(sqlx::Error::Io) })
            })
            .connect_with(options)
            .await?;
        sqlx::migrate!("./migrations").run(&pool).await?;
        restrict(path)?;
        Ok(Self { pool })
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
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

    pub async fn extend_device(&self, id: &str, expires_at: Option<i64>) -> Result<Extension> {
        let mut tx = self.pool.begin().await?;
        let found = sqlx::query("UPDATE devices SET expires_at = ? WHERE id = ?")
            .bind(expires_at)
            .bind(id)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        // Every live token, not the newest: a device that paired more than once holds whichever
        // one reached it and the store cannot tell which. Revocation is the per-token kill switch
        // and renewal must not undo it. One transaction, because a device row extended with its
        // token left behind is the exact defect this fixes.
        let tokens =
            sqlx::query("UPDATE tokens SET expires_at = ? WHERE device_id = ? AND revoked_at IS NULL")
                .bind(expires_at)
                .bind(id)
                .execute(&mut *tx)
                .await?
                .rows_affected();
        tx.commit().await?;
        Ok(Extension { found, tokens })
    }

    /// `armed_until` is when the code stops being redeemable. `None` means it never starts:
    /// somebody at the console has to arm it first.
    pub async fn create_pairing(
        &self,
        role: Role,
        now: i64,
        expires_at: i64,
        armed_until: Option<i64>,
    ) -> Result<String> {
        let code = secret::pairing_code()?;
        sqlx::query(
            "INSERT INTO pairings (hash, role, created_at, expires_at, armed_until)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(secret::pairing_digest(&secret::normalise_code(&code)))
        .bind(role.as_str())
        .bind(now)
        .bind(expires_at)
        .bind(armed_until)
        .execute(&self.pool)
        .await?;
        Ok(code)
    }

    pub async fn arm_pairing(&self, code: &str, now: i64, until: i64) -> Result<bool> {
        let done = sqlx::query(
            "UPDATE pairings SET armed_until = ?
             WHERE hash = ? AND used_at IS NULL AND expires_at > ? AND attempts < ?",
        )
        .bind(until)
        .bind(secret::pairing_digest(&secret::normalise_code(code)))
        .bind(now)
        .bind(PAIRING_ATTEMPT_LIMIT)
        .execute(&self.pool)
        .await?;
        Ok(done.rows_affected() > 0)
    }

    /// Single use: the row is marked spent in the same statement that claims it, so two devices
    /// racing on one code cannot both win.
    ///
    /// A miss charges every outstanding code an attempt. That is deliberately blunt — a wrong
    /// guess matches no row, so there is no other row to charge — and it means an attacker can
    /// burn a pending code with ten guesses. Burning one costs the operator a re-print; not
    /// counting at all costs them the code.
    pub async fn claim_pairing(&self, code: &str, now: i64) -> Result<Option<Role>> {
        let hash = secret::pairing_digest(&secret::normalise_code(code));
        let mut tx = self.pool.begin().await?;
        let row = sqlx::query(
            "SELECT role FROM pairings
             WHERE hash = ? AND used_at IS NULL AND expires_at > ? AND attempts < ?
               AND armed_until IS NOT NULL AND armed_until > ?",
        )
        .bind(&hash)
        .bind(now)
        .bind(PAIRING_ATTEMPT_LIMIT)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some(row) = row else {
            sqlx::query(
                "UPDATE pairings SET attempts = attempts + 1 WHERE used_at IS NULL AND expires_at > ?",
            )
            .bind(now)
            .execute(&mut *tx)
            .await?;
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

    /// Mints a recovery code and retires any that came before it, so there is never more than
    /// one live way back in. Used rows are kept: what a recovery code was spent on is exactly the
    /// thing an operator wants to be able to look up afterwards.
    pub async fn issue_recovery(&self, now: i64) -> Result<String> {
        let code = secret::recovery_code()?;
        let mut tx = self.pool.begin().await?;
        sqlx::query("DELETE FROM recovery_codes WHERE used_at IS NULL")
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO recovery_codes (hash, created_at) VALUES (?, ?)")
            .bind(secret::recovery_digest(&secret::normalise_code(&code)))
            .bind(now)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(code)
    }

    /// Single use, claimed and spent in one transaction. A miss is counted and nothing more: a
    /// limit that killed the code would hand an attacker a permanent lockout for the price of a
    /// few wrong guesses, and at ~99 bits there is nothing to guess.
    pub async fn claim_recovery(&self, code: &str, now: i64) -> Result<bool> {
        let hash = secret::recovery_digest(&secret::normalise_code(code));
        let mut tx = self.pool.begin().await?;
        let claimed = sqlx::query("UPDATE recovery_codes SET used_at = ? WHERE hash = ? AND used_at IS NULL")
            .bind(now)
            .bind(&hash)
            .execute(&mut *tx)
            .await?
            .rows_affected()
            > 0;
        if !claimed {
            sqlx::query("UPDATE recovery_codes SET attempts = attempts + 1 WHERE used_at IS NULL")
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        Ok(claimed)
    }

    pub async fn has_recovery(&self) -> Result<bool> {
        Ok(self.recovery_issued_at().await?.is_some())
    }

    pub async fn recovery_issued_at(&self) -> Result<Option<i64>> {
        let row = sqlx::query("SELECT created_at FROM recovery_codes WHERE used_at IS NULL")
            .fetch_optional(&self.pool)
            .await?;
        Ok(row.map(|r| r.get("created_at")))
    }

    pub async fn recovery_attempts(&self) -> Result<i64> {
        let row =
            sqlx::query("SELECT COALESCE(SUM(attempts), 0) AS n FROM recovery_codes WHERE used_at IS NULL")
                .fetch_one(&self.pool)
                .await?;
        Ok(row.get("n"))
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
        // Pane ids change whenever herdr restarts, so validating the id against the live herd
        // still leaves a set that only grows. This is the floor under it.
        sqlx::query(
            "DELETE FROM pane_prefs WHERE device_id = ?1 AND pane_id NOT IN
             (SELECT pane_id FROM pane_prefs WHERE device_id = ?1 ORDER BY updated_at DESC LIMIT ?2)",
        )
        .bind(device_id)
        .bind(MAX_PANE_PREFS)
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
    for sidecar in ["", "-wal", "-shm"] {
        let mut candidate = path.as_os_str().to_os_string();
        candidate.push(sidecar);
        let candidate = std::path::PathBuf::from(candidate);
        if candidate.exists() {
            chmod(&candidate, 0o600)?;
        }
    }
    Ok(())
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
    }

    /// The defect this guards: the device row was extended and the token it authenticates with was
    /// not, so renew reported success and the device still had to pair again.
    #[tokio::test]
    async fn extending_a_device_carries_the_token_it_is_already_holding_with_it() {
        let s = store().await;
        let d = s
            .create_device("phone", Role::Full, NOW, Some(NOW + 10), None, None)
            .await
            .unwrap();
        let token = s.mint_token(&d.id, NOW, Some(NOW + 10)).await.unwrap();
        assert_eq!(s.device_for_token(&token, NOW + 11).await.unwrap(), None);

        let extended = s.extend_device(&d.id, Some(NOW + 100)).await.unwrap();
        assert!(extended.found);
        assert_eq!(extended.tokens, 1);
        assert_eq!(
            s.device_for_token(&token, NOW + 11).await.unwrap().map(|d| d.id),
            Some(d.id),
            "the operator pressed Renew and the device still cannot connect"
        );
    }

    /// A device that re-paired holds whichever token reached it; the store cannot tell which, so
    /// renewal covers every live one. Revocation is the per-token kill switch and it stays.
    #[tokio::test]
    async fn extending_a_device_renews_every_live_token_and_resurrects_no_revoked_one() {
        let s = store().await;
        let d = s
            .create_device("phone", Role::Full, NOW, Some(NOW + 10), None, None)
            .await
            .unwrap();
        let first = s.mint_token(&d.id, NOW, Some(NOW + 10)).await.unwrap();
        let second = s.mint_token(&d.id, NOW + 1, Some(NOW + 10)).await.unwrap();
        let dead = s.mint_token(&d.id, NOW + 2, Some(NOW + 10)).await.unwrap();
        sqlx::query("UPDATE tokens SET revoked_at = ? WHERE hash = ?")
            .bind(NOW + 3)
            .bind(secret::digest(&dead))
            .execute(s.pool())
            .await
            .unwrap();

        let extended = s.extend_device(&d.id, Some(NOW + 100)).await.unwrap();
        assert_eq!(extended.tokens, 2);
        assert!(s.device_for_token(&first, NOW + 11).await.unwrap().is_some());
        assert!(s.device_for_token(&second, NOW + 11).await.unwrap().is_some());
        assert_eq!(
            s.device_for_token(&dead, NOW + 11).await.unwrap(),
            None,
            "a revoked token came back"
        );
    }

    #[tokio::test]
    async fn extending_a_device_that_never_expires_invents_no_expiry() {
        let s = store().await;
        let d = s
            .create_device("phone", Role::Full, NOW, None, None, None)
            .await
            .unwrap();
        let token = s.mint_token(&d.id, NOW, None).await.unwrap();
        assert_eq!(s.extend_device(&d.id, None).await.unwrap().tokens, 1);
        assert_eq!(s.device(&d.id).await.unwrap().unwrap().expires_at, None);
        assert!(
            s.device_for_token(&token, NOW + 10_000_000)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn extending_a_device_that_is_not_there_extends_nothing() {
        let s = store().await;
        let missing = s.extend_device("nope", Some(NOW + 100)).await.unwrap();
        assert!(!missing.found);
        assert_eq!(missing.tokens, 0);
    }

    #[tokio::test]
    async fn a_pairing_code_is_single_use_and_time_boxed() {
        let s = store().await;
        let code = s
            .create_pairing(Role::Readonly, NOW, NOW + 600, Some(NOW + 600))
            .await
            .unwrap();
        assert_eq!(s.claim_pairing(&code, NOW).await.unwrap(), Some(Role::Readonly));
        assert_eq!(s.claim_pairing(&code, NOW).await.unwrap(), None);

        let stale = s
            .create_pairing(Role::Full, NOW, NOW + 600, Some(NOW + 600))
            .await
            .unwrap();
        assert_eq!(s.claim_pairing(&stale, NOW + 601).await.unwrap(), None);
    }

    #[tokio::test]
    async fn an_unarmed_code_is_not_a_credential() {
        let s = store().await;
        let code = s.create_pairing(Role::Full, NOW, NOW + 600, None).await.unwrap();
        assert_eq!(s.claim_pairing(&code, NOW).await.unwrap(), None);
        assert!(s.arm_pairing(&code, NOW, NOW + 60).await.unwrap());
        assert_eq!(
            s.claim_pairing(&code, NOW + 61).await.unwrap(),
            None,
            "the window closes"
        );
        assert!(!s.arm_pairing("ZZZZ-ZZZZ", NOW, NOW + 60).await.unwrap());
    }

    #[tokio::test]
    async fn a_run_of_wrong_guesses_burns_every_outstanding_code() {
        let s = store().await;
        let code = s
            .create_pairing(Role::Full, NOW, NOW + 600, Some(NOW + 600))
            .await
            .unwrap();
        for _ in 0..PAIRING_ATTEMPT_LIMIT {
            assert_eq!(s.claim_pairing("ZZZZ-ZZZZ", NOW).await.unwrap(), None);
        }
        assert_eq!(
            s.claim_pairing(&code, NOW).await.unwrap(),
            None,
            "the attempts column is what makes a 40-bit code safe; it has to be written"
        );
    }

    #[tokio::test]
    async fn a_pairing_code_matches_however_it_was_typed() {
        let s = store().await;
        let code = s
            .create_pairing(Role::Full, NOW, NOW + 600, Some(NOW + 600))
            .await
            .unwrap();
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
    #[cfg(unix)]
    async fn the_state_directory_and_every_sidecar_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("state");
        let path = dir.join("kampr.db");
        let s = Store::open(&path).await.unwrap();
        // A pairing digest lives in the write-ahead log long before it reaches the main file.
        s.create_pairing(Role::Full, NOW, NOW + 600, Some(NOW + 600))
            .await
            .unwrap();

        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700, "the state directory");
        for sidecar in ["kampr.db", "kampr.db-wal", "kampr.db-shm"] {
            let p = dir.join(sidecar);
            assert!(p.exists(), "{sidecar} should exist");
            assert_eq!(mode(&p), 0o600, "{sidecar} carries pairing digests");
        }
    }

    /// `restrict` runs once, at open. Every connection opened after that can find sidecars sqlite
    /// recreated for itself at the process umask, and the one that opens them is the one that has
    /// to tighten them.
    #[tokio::test]
    #[cfg(unix)]
    async fn a_sidecar_that_comes_back_at_the_umask_is_made_private_by_the_connection_that_opens_it() {
        use std::os::unix::fs::PermissionsExt;
        let parent = tempfile::tempdir().unwrap();
        let dir = parent.path().join("state");
        let path = dir.join("kampr.db");
        let s = Store::open(&path).await.unwrap();
        s.create_pairing(Role::Full, NOW, NOW + 600, Some(NOW + 600))
            .await
            .unwrap();

        let wal = dir.join("kampr.db-wal");
        chmod(&wal, 0o644).unwrap();

        // Enough handles at once that the pool has to open a connection it did not already hold.
        let mut held = Vec::new();
        for _ in 0..s.pool().options().get_max_connections() {
            held.push(s.pool().acquire().await.unwrap());
        }
        drop(held);

        let mode = std::fs::metadata(&wal).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "a pairing digest was left world-readable in the write-ahead log"
        );
    }

    /// Asserted as configuration rather than measured: the defect is a ten-minute idle timeout and
    /// a thirty-minute lifetime, and a test cannot wait either of them out. What it buys is that
    /// the last connection never closes, so sqlite never unlinks `-wal` and never recreates it at
    /// whatever umask the node happens to have.
    #[tokio::test]
    async fn the_pool_never_drops_to_zero_connections_and_so_never_tears_the_write_ahead_log_down() {
        let parent = tempfile::tempdir().unwrap();
        let path = parent.path().join("state").join("kampr.db");
        let s = Store::open(&path).await.unwrap();
        let options = s.pool().options();
        assert!(
            options.get_min_connections() >= 1,
            "the pool empties itself when idle"
        );
        assert_eq!(
            options.get_idle_timeout(),
            None,
            "an idle connection is still closed"
        );
        assert_eq!(
            options.get_max_lifetime(),
            None,
            "a live connection is still retired"
        );
    }

    #[tokio::test]
    async fn a_device_cannot_keep_preferences_for_more_panes_than_the_cap() {
        let s = store().await;
        let d = s
            .create_device("phone", Role::Full, NOW, None, None, None)
            .await
            .unwrap();
        for n in 0..(MAX_PANE_PREFS + 20) {
            s.set_pane_prefs(
                &d.id,
                &format!("n/w1:p{n}"),
                &serde_json::json!({ "zoom": 1 }),
                NOW + n,
            )
            .await
            .unwrap();
        }
        let kept = s.pane_prefs(&d.id).await.unwrap();
        assert_eq!(kept.as_object().unwrap().len(), MAX_PANE_PREFS as usize);
        assert!(kept.get("n/w1:p0").is_none(), "the oldest go first");
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

    #[tokio::test]
    async fn a_recovery_code_is_single_use_and_replaces_the_one_before_it() {
        let s = store().await;
        assert!(!s.has_recovery().await.unwrap());

        let first = s.issue_recovery(NOW).await.unwrap();
        assert!(s.has_recovery().await.unwrap());
        let second = s.issue_recovery(NOW + 1).await.unwrap();
        assert_ne!(first, second);
        assert!(
            !s.claim_recovery(&first, NOW + 2).await.unwrap(),
            "issuing a new code retires the old one"
        );

        assert!(s.claim_recovery(&second, NOW + 3).await.unwrap());
        assert!(!s.claim_recovery(&second, NOW + 4).await.unwrap(), "single use");
        assert!(!s.has_recovery().await.unwrap());
    }

    #[tokio::test]
    async fn a_recovery_code_matches_however_it_was_typed() {
        let s = store().await;
        let code = s.issue_recovery(NOW).await.unwrap();
        let typed = code.replace('-', " ").to_lowercase();
        assert!(s.claim_recovery(&typed, NOW).await.unwrap());
    }

    /// Deliberately unlike the pairing code: wrong guesses are counted and shown, never fatal.
    /// Burning the last way back into a host costs ten wrong guesses if it is fatal, and the
    /// entropy already puts guessing out of reach.
    #[tokio::test]
    async fn wrong_guesses_are_counted_but_never_burn_the_code() {
        let s = store().await;
        let code = s.issue_recovery(NOW).await.unwrap();
        for _ in 0..(PAIRING_ATTEMPT_LIMIT * 5) {
            assert!(!s.claim_recovery("ZZZZ-ZZZZ-ZZZZ-ZZZZ-ZZZZ", NOW).await.unwrap());
        }
        assert_eq!(s.recovery_attempts().await.unwrap(), PAIRING_ATTEMPT_LIMIT * 5);
        assert!(
            s.claim_recovery(&code, NOW).await.unwrap(),
            "a guessing attacker must not be able to lock the operator out for good"
        );
    }
}

#[cfg(test)]
mod role_tests {
    use super::*;

    #[test]
    fn an_unreadable_role_fails_closed() {
        assert_eq!(Role::default(), Role::Readonly);
        assert_eq!("nonsense".parse::<Role>().unwrap_or_default(), Role::Readonly);
        assert_eq!("".parse::<Role>().unwrap_or_default(), Role::Readonly);
        assert_eq!("FULL".parse::<Role>().unwrap_or_default(), Role::Readonly);
        assert_eq!("full".parse::<Role>().unwrap(), Role::Full);
    }
}
