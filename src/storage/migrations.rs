mod document_paths;

use super::Store;
use crate::library::{Id, Session, local_path};
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut conn =
            Connection::open(path).context(crate::i18n::tr("Не удалось открыть библиотеку"))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        let version: i64 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .context("Read library schema version")?;
        if version > 5 {
            bail!(tformat!(
                "Библиотека создана более новой версией Carlitos (схема {version})",
                version = version
            ));
        }
        configure_connection(&conn).context("Configure library connection")?;
        if version < 4 {
            // A second opener can wait here while another connection initializes
            // or upgrades the library. Read the schema only after owning the
            // write lock, then commit all legacy steps together.
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
            anyhow::ensure!(version <= 5, "Library schema changed during migration");
            if version == 0 {
                tx.execute_batch(include_str!("schema.sql"))?;
                tx.pragma_update(None, "user_version", 1)?;
            }
            if version < 2 {
                tx.execute_batch("CREATE TABLE source_files(source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,file_id INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,relative TEXT NOT NULL,PRIMARY KEY(source_id,file_id),UNIQUE(source_id,relative)); INSERT INTO source_files SELECT source_id,id,relative FROM media_files;")?;
                let sources: Vec<(Id, String)> = tx
                    .prepare("SELECT id,uri FROM sources")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                let files: Vec<(Id, String)> = tx
                    .prepare("SELECT id,uri FROM media_files")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                for (source, uri) in sources {
                    if let Some(root) = local_path(&uri) {
                        for (file, uri) in &files {
                            if let Some(path) = local_path(uri)
                                && let Ok(relative) = path.strip_prefix(&root)
                            {
                                tx.execute(
                                    "INSERT OR IGNORE INTO source_files VALUES(?1,?2,?3)",
                                    params![source, file, relative.to_string_lossy()],
                                )?;
                            }
                        }
                    }
                }
                tx.pragma_update(None, "user_version", 2)?;
            }
            if version < 3 {
                // Keep legacy media/source records so shared book files retain their IDs.
                // Only the obsolete queue and music flags are removed.
                tx.execute_batch(
                    "DROP TABLE queue_entries; ALTER TABLE media_files DROP COLUMN music;",
                )?;
                let state: Option<String> = tx
                    .query_row("SELECT data FROM session_state WHERE id=1", [], |r| {
                        r.get(0)
                    })
                    .optional()?;
                if let Some(state) = state {
                    let mut value: serde_json::Value = serde_json::from_str(&state)?;
                    if value["current"].get("Music").is_some() {
                        value["current"] = serde_json::Value::Null;
                        value["position"] = 0.into();
                    }
                    let session: Session = serde_json::from_value(value)?;
                    tx.execute(
                        "UPDATE session_state SET data=?1 WHERE id=1",
                        [serde_json::to_string(&session)?],
                    )?;
                }
                tx.pragma_update(None, "user_version", 3)?;
            }
            if version < 4 {
                // Reimports look up each recording and replace its chapters.
                // Index child keys so these operations and FK checks do not scan
                // the entire library once per file.
                tx.execute_batch(
                    "CREATE INDEX book_parts_file ON book_parts(file_id);
                     CREATE INDEX chapters_part ON chapters(part_id);
                     CREATE INDEX source_files_file ON source_files(file_id);",
                )?;
                tx.pragma_update(None, "user_version", 4)?;
            }
            tx.commit()?;
        }
        if version < 5 {
            document_paths::migrate(&mut conn)?;
        }
        Ok(Self { conn })
    }
}

fn configure_connection(conn: &Connection) -> rusqlite::Result<()> {
    use std::time::{Duration, Instant};

    conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=FULL;")?;
    let started = Instant::now();
    loop {
        match conn.pragma_update(None, "journal_mode", "WAL") {
            // Competing connections can both hold a read lock before changing
            // the journal mode. SQLite returns BUSY without the busy handler
            // to avoid deadlock. Each failed statement releases its lock, so
            // retry this setup outside a transaction with a bounded retry window.
            Err(error)
                if error.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)
                    && started.elapsed() < Duration::from_secs(5) =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            result => return result,
        }
    }
}
