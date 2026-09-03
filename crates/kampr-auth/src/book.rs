//! The fleet book: commands the operator ran across the herd, and the ones they kept.
//!
//! In the database that already owns devices, but **not keyed by a device**. Every other table
//! here hangs off `device_id`, and that grain is wrong for this one: a device row is the identity
//! in this schema — there is no account above it — so a phone and a desktop are two rows, and a
//! book keyed by device is empty on whichever one the operator picks up second. The whole of the
//! request was that the list follow them between devices, and the node is the grain that does.

use crate::store::{Store, StoreError};
use serde::Serialize;
use sqlx::Row;

type Result<T> = std::result::Result<T, StoreError>;

/// History entries kept. Small on purpose: a list long enough to scroll is a list nobody reads,
/// and Saved is where a command earns permanence.
pub const FLEET_RECENT: usize = 5;

/// Saved entries kept. A bound rather than a considered limit — the book has no per-device
/// scoping, so any writer can otherwise fill the disk one command at a time.
pub const MAX_FLEET_SAVED: i64 = 64;

/// The largest argv the book will hold, measured on its canonical key.
pub const MAX_FLEET_KEY_BYTES: usize = 4096;

pub const KIND_RECENT: &str = "recent";
pub const KIND_SAVED: &str = "saved";

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FleetCommand {
    pub id: String,
    pub args: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub at: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct FleetBook {
    pub recent: Vec<FleetCommand>,
    pub saved: Vec<FleetCommand>,
}

/// What makes two runs the same command.
///
/// The argv **and** the working directory, because `make` in two trees is two commands. Nothing
/// about which hosts it reached is in here: a run is fanned out to every host reachable at the
/// time, that set is different next week, and a saved entry that pinned last week's would offer
/// to run somewhere that no longer exists.
fn key_of(args: &[String], cwd: Option<&str>) -> String {
    serde_json::json!({ "args": args, "cwd": cwd }).to_string()
}

fn row_to_command(row: &sqlx::sqlite::SqliteRow) -> FleetCommand {
    FleetCommand {
        id: row.get::<String, _>("id"),
        args: serde_json::from_str(&row.get::<String, _>("args")).unwrap_or_default(),
        cwd: row.get::<Option<String>, _>("cwd"),
        label: row.get::<Option<String>, _>("label"),
        at: row.get::<i64, _>("at"),
    }
}

impl Store {
    pub async fn fleet_book(&self) -> Result<FleetBook> {
        let rows = sqlx::query("SELECT id, kind, args, cwd, label, at FROM fleet_commands ORDER BY seq DESC")
            .fetch_all(self.pool())
            .await?;
        let mut book = FleetBook::default();
        for row in &rows {
            match row.get::<String, _>("kind").as_str() {
                KIND_SAVED => book.saved.push(row_to_command(row)),
                _ => book.recent.push(row_to_command(row)),
            }
        }
        book.recent.truncate(FLEET_RECENT);
        Ok(book)
    }

    /// A run the operator issued, remembered.
    ///
    /// **A command already in the book is moved, never copied** — and if that entry is a saved
    /// one it stays saved, so promoting a command to Saved and then running it does not put it in
    /// both lists.
    pub async fn record_fleet_run(&self, args: &[String], cwd: Option<&str>, now: i64) -> Result<bool> {
        let key = key_of(args, cwd);
        if args.is_empty() || key.len() > MAX_FLEET_KEY_BYTES {
            return Ok(false);
        }
        let touched = sqlx::query(
            "UPDATE fleet_commands SET at = ?1, seq = (SELECT IFNULL(MAX(seq), 0) + 1 FROM fleet_commands)
             WHERE key = ?2",
        )
        .bind(now)
        .bind(&key)
        .execute(self.pool())
        .await?
        .rows_affected();
        if touched == 0 {
            sqlx::query(
                "INSERT INTO fleet_commands (id, key, kind, args, cwd, label, at, seq)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6,
                         (SELECT IFNULL(MAX(seq), 0) + 1 FROM fleet_commands))",
            )
            .bind(ulid::Ulid::generate().to_string())
            .bind(&key)
            .bind(KIND_RECENT)
            .bind(serde_json::to_string(args).unwrap_or_default())
            .bind(cwd)
            .bind(now)
            .execute(self.pool())
            .await?;
        }
        self.trim_recent().await?;
        Ok(true)
    }

    /// Keeps a command, by argv or by promoting one already in the book.
    ///
    /// Promotion is a move: the row keeps its id and changes kind, so a command cannot end up in
    /// the history and in Saved at once.
    pub async fn save_fleet_command(
        &self,
        args: &[String],
        cwd: Option<&str>,
        label: Option<&str>,
        now: i64,
    ) -> Result<Option<FleetCommand>> {
        let key = key_of(args, cwd);
        if args.is_empty() || key.len() > MAX_FLEET_KEY_BYTES {
            return Ok(None);
        }
        let saved: i64 =
            sqlx::query_scalar("SELECT count(*) FROM fleet_commands WHERE kind = ?1 AND key <> ?2")
                .bind(KIND_SAVED)
                .bind(&key)
                .fetch_one(self.pool())
                .await?;
        if saved >= MAX_FLEET_SAVED {
            return Ok(None);
        }
        sqlx::query(
            "INSERT INTO fleet_commands (id, key, kind, args, cwd, label, at, seq)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7,
                     (SELECT IFNULL(MAX(seq), 0) + 1 FROM fleet_commands))
             ON CONFLICT(key) DO UPDATE SET kind = ?3, label = ?6, at = ?7,
                     seq = (SELECT IFNULL(MAX(seq), 0) + 1 FROM fleet_commands)",
        )
        .bind(ulid::Ulid::generate().to_string())
        .bind(&key)
        .bind(KIND_SAVED)
        .bind(serde_json::to_string(args).unwrap_or_default())
        .bind(cwd)
        .bind(label.map(str::trim).filter(|l| !l.is_empty()))
        .bind(now)
        .execute(self.pool())
        .await?;
        self.fleet_command(&key).await
    }

    /// Promotes an entry the operator can already see, keeping its id.
    ///
    /// By id rather than by argv because that is what the operator pressed: re-deriving the key
    /// from a command the client re-typed is a second chance to disagree about what "the same
    /// command" is.
    pub async fn keep_fleet_command(&self, id: &str, label: Option<&str>, now: i64) -> Result<bool> {
        let saved: i64 =
            sqlx::query_scalar("SELECT count(*) FROM fleet_commands WHERE kind = ?1 AND id <> ?2")
                .bind(KIND_SAVED)
                .bind(id)
                .fetch_one(self.pool())
                .await?;
        if saved >= MAX_FLEET_SAVED {
            return Ok(false);
        }
        let changed = sqlx::query(
            "UPDATE fleet_commands SET kind = ?1, label = ?2, at = ?3,
             seq = (SELECT IFNULL(MAX(seq), 0) + 1 FROM fleet_commands) WHERE id = ?4",
        )
        .bind(KIND_SAVED)
        .bind(label.map(str::trim).filter(|l| !l.is_empty()))
        .bind(now)
        .bind(id)
        .execute(self.pool())
        .await?
        .rows_affected();
        Ok(changed > 0)
    }

    /// Removes one entry, whichever list it is in. The one thing the operator can always do about
    /// a command they did not want written down.
    pub async fn drop_fleet_command(&self, id: &str) -> Result<bool> {
        let gone = sqlx::query("DELETE FROM fleet_commands WHERE id = ?1")
            .bind(id)
            .execute(self.pool())
            .await?
            .rows_affected();
        Ok(gone > 0)
    }

    async fn fleet_command(&self, key: &str) -> Result<Option<FleetCommand>> {
        let row = sqlx::query("SELECT id, kind, args, cwd, label, at FROM fleet_commands WHERE key = ?1")
            .bind(key)
            .fetch_optional(self.pool())
            .await?;
        Ok(row.as_ref().map(row_to_command))
    }

    async fn trim_recent(&self) -> Result<()> {
        sqlx::query(
            "DELETE FROM fleet_commands WHERE kind = ?1 AND id NOT IN
             (SELECT id FROM fleet_commands WHERE kind = ?1 ORDER BY seq DESC LIMIT ?2)",
        )
        .bind(KIND_RECENT)
        .bind(FLEET_RECENT as i64)
        .execute(self.pool())
        .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_774_000_000;

    fn argv(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_string()).collect()
    }

    async fn store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("a dir");
        let store = Store::open(&dir.path().join("kampr.db")).await.expect("a store");
        (dir, store)
    }

    #[tokio::test]
    async fn a_command_run_twice_moves_up_instead_of_appearing_twice() {
        let (_dir, store) = store().await;
        store
            .record_fleet_run(&argv(&["pacman", "-Syu"]), None, NOW)
            .await
            .unwrap();
        store
            .record_fleet_run(&argv(&["uptime"]), None, NOW + 1)
            .await
            .unwrap();
        store
            .record_fleet_run(&argv(&["pacman", "-Syu"]), None, NOW + 2)
            .await
            .unwrap();
        let book = store.fleet_book().await.unwrap();
        assert_eq!(book.recent.len(), 2);
        assert_eq!(book.recent[0].args, argv(&["pacman", "-Syu"]));
    }

    #[tokio::test]
    async fn the_same_argv_in_two_directories_is_two_commands() {
        let (_dir, store) = store().await;
        store
            .record_fleet_run(&argv(&["make"]), Some("/a"), NOW)
            .await
            .unwrap();
        store
            .record_fleet_run(&argv(&["make"]), Some("/b"), NOW + 1)
            .await
            .unwrap();
        assert_eq!(store.fleet_book().await.unwrap().recent.len(), 2);
    }

    #[tokio::test]
    async fn history_keeps_five_and_drops_the_oldest() {
        let (_dir, store) = store().await;
        for n in 0..(FLEET_RECENT + 3) {
            store
                .record_fleet_run(&argv(&["echo", &n.to_string()]), None, NOW + n as i64)
                .await
                .unwrap();
        }
        let book = store.fleet_book().await.unwrap();
        assert_eq!(book.recent.len(), FLEET_RECENT);
        assert_eq!(book.recent[0].args, argv(&["echo", "7"]));
        assert_eq!(book.recent[FLEET_RECENT - 1].args, argv(&["echo", "3"]));
    }

    /// The defect this guards is a command listed twice under two headings, which is the one thing
    /// the operator would read as the book having lost track of itself.
    #[tokio::test]
    async fn a_promoted_command_leaves_the_history_and_stays_out_of_it_when_run_again() {
        let (_dir, store) = store().await;
        store
            .record_fleet_run(&argv(&["kampr", "update"]), None, NOW)
            .await
            .unwrap();
        let id = store.fleet_book().await.unwrap().recent[0].id.clone();
        assert!(
            store
                .keep_fleet_command(&id, Some("update everything"), NOW + 1)
                .await
                .unwrap()
        );

        let book = store.fleet_book().await.unwrap();
        assert!(
            book.recent.is_empty(),
            "a promoted command was left in the history"
        );
        assert_eq!(book.saved[0].label.as_deref(), Some("update everything"));
        assert_eq!(book.saved[0].id, id, "promotion minted a second row");

        store
            .record_fleet_run(&argv(&["kampr", "update"]), None, NOW + 2)
            .await
            .unwrap();
        let book = store.fleet_book().await.unwrap();
        assert!(
            book.recent.is_empty(),
            "running a saved command put it in the history too"
        );
        assert_eq!(book.saved.len(), 1);
    }

    /// A fan-out is several runs inside one wall-clock second, and `at` is a second. Ordering on
    /// it alone left the list in whatever order SQLite felt like and the trim keeping an arbitrary
    /// five — so the operator's newest command was not at the top and sometimes was not there.
    #[tokio::test]
    async fn commands_run_inside_one_second_are_still_newest_first() {
        let (_dir, store) = store().await;
        for n in 0..(FLEET_RECENT + 3) {
            store
                .record_fleet_run(&argv(&["echo", &n.to_string()]), None, NOW)
                .await
                .unwrap();
        }
        let book = store.fleet_book().await.unwrap();
        assert_eq!(
            book.recent.iter().map(|c| c.args[1].clone()).collect::<Vec<_>>(),
            vec!["7", "6", "5", "4", "3"],
        );
    }

    #[tokio::test]
    async fn re_running_an_old_command_in_the_same_second_still_moves_it_to_the_top() {
        let (_dir, store) = store().await;
        store
            .record_fleet_run(&argv(&["first"]), None, NOW)
            .await
            .unwrap();
        store
            .record_fleet_run(&argv(&["second"]), None, NOW)
            .await
            .unwrap();
        store
            .record_fleet_run(&argv(&["first"]), None, NOW)
            .await
            .unwrap();
        let book = store.fleet_book().await.unwrap();
        assert_eq!(book.recent[0].args, argv(&["first"]));
    }

    #[tokio::test]
    async fn saving_the_same_command_twice_relabels_one_entry() {
        let (_dir, store) = store().await;
        store
            .save_fleet_command(&argv(&["uptime"]), None, Some("who is up"), NOW)
            .await
            .unwrap();
        store
            .save_fleet_command(&argv(&["uptime"]), None, Some("load"), NOW + 1)
            .await
            .unwrap();
        let book = store.fleet_book().await.unwrap();
        assert_eq!(book.saved.len(), 1);
        assert_eq!(book.saved[0].label.as_deref(), Some("load"));
    }

    #[tokio::test]
    async fn anything_in_the_book_can_be_deleted() {
        let (_dir, store) = store().await;
        store
            .record_fleet_run(&argv(&["TOKEN=hunter2", "./deploy"]), None, NOW)
            .await
            .unwrap();
        let saved = store
            .save_fleet_command(&argv(&["uptime"]), None, None, NOW)
            .await
            .unwrap()
            .expect("a saved entry");
        let recent = store.fleet_book().await.unwrap().recent[0].id.clone();
        assert!(store.drop_fleet_command(&recent).await.unwrap());
        assert!(store.drop_fleet_command(&saved.id).await.unwrap());
        assert_eq!(store.fleet_book().await.unwrap(), FleetBook::default());
        assert!(!store.drop_fleet_command(&recent).await.unwrap());
    }

    #[tokio::test]
    async fn the_saved_list_is_bounded() {
        let (_dir, store) = store().await;
        for n in 0..MAX_FLEET_SAVED {
            assert!(
                store
                    .save_fleet_command(&argv(&["echo", &n.to_string()]), None, None, NOW + n)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        assert!(
            store
                .save_fleet_command(&argv(&["echo", "over"]), None, None, NOW)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            store.fleet_book().await.unwrap().saved.len(),
            MAX_FLEET_SAVED as usize
        );
    }
}
