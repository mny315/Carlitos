use anyhow::{Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

/// Paths describe a source listing, not a permanent recording identity. Keep
/// historical records when another document later occupies their old name.
pub(super) fn migrate(conn: &mut Connection) -> Result<()> {
    // Rebuild parent tables using SQLite's create/copy/drop/rename procedure.
    // Disabling FKs outside the transaction avoids cascading into saved parts.
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = (|| -> Result<()> {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(version <= 5, "Library schema changed during migration");
        if version == 5 {
            // Another connection may have migrated while we waited for its lock.
            return Ok(());
        }
        ensure!(version == 4, "Expected library schema 4");
        let objects: Vec<String> = tx
            .prepare(
                "SELECT sql FROM sqlite_schema
                 WHERE tbl_name IN ('media_files','source_files')
                   AND type IN ('index','trigger') AND sql IS NOT NULL",
            )?
            .query_map([], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        tx.execute_batch(include_str!("document_paths.sql"))?;
        for sql in objects {
            tx.execute_batch(&sql)?;
        }
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS media_files_source_path ON media_files(source_id,relative);",
        )?;
        let violation: Option<String> = tx
            .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
            .optional()?;
        ensure!(
            violation.is_none(),
            "Invalid library references: {violation:?}"
        );
        tx.pragma_update(None, "user_version", 5)?;
        tx.commit()?;
        Ok(())
    })();
    // Also restore enforcement after a failed copy or integrity check.
    let restored = conn.pragma_update(None, "foreign_keys", true);
    result?;
    restored?;
    Ok(())
}
