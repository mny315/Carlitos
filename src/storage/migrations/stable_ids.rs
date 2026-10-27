use anyhow::{Context, Result, ensure};
use rusqlite::{Connection, OptionalExtension, TransactionBehavior};

/// UI actions and background results can outlive a deleted library entry.
/// Never let a new source, file, book or part inherit that entry's ID.
pub(super) fn migrate(conn: &mut Connection) -> Result<()> {
    let legacy_alter: bool = conn.pragma_query_value(None, "legacy_alter_table", |r| r.get(0))?;
    conn.pragma_update(None, "foreign_keys", false)?;
    let result = (|| -> Result<()> {
        // Existing views/triggers may reference a table while its replacement
        // is temporarily under another name. Keep their original references.
        conn.pragma_update(None, "legacy_alter_table", true)?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(version <= 8, "Library schema changed during migration");
        if version == 8 {
            return Ok(());
        }
        ensure!(version == 7, "Expected library schema 7");
        let mut tables = Vec::new();
        let mut objects = Vec::<String>::new();
        for table in ["sources", "media_files", "books", "book_parts"] {
            let sql: String = tx.query_row(
                "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
                [table],
                |r| r.get(0),
            )?;
            let (_, columns) = sql
                .split_once('(')
                .context("Invalid library table definition")?;
            let key = columns
                .trim_start()
                .strip_prefix("id INTEGER PRIMARY KEY")
                .with_context(|| format!("Unrecognized primary key in {table}"))?;
            if key.trim_start().starts_with("AUTOINCREMENT") {
                continue;
            }
            let definition = columns.replacen(
                "id INTEGER PRIMARY KEY",
                "id INTEGER PRIMARY KEY AUTOINCREMENT",
                1,
            );
            ensure!(definition != columns, "Unrecognized primary key in {table}");
            objects.extend(
                tx.prepare(
                    "SELECT sql FROM sqlite_schema WHERE tbl_name=?1
                 AND type IN ('index','trigger') AND sql IS NOT NULL ORDER BY type,name",
                )?
                .query_map([table], |r| r.get::<_, String>(0))?
                .collect::<Result<Vec<_>, _>>()?,
            );
            // Names are fixed above, never derived from user metadata. Copy
            // original columns/constraints and explicit IDs without firing
            // progress or audit triggers during migration.
            tx.execute_batch(&format!(
                "CREATE TABLE {table}_v8({definition};
                 INSERT INTO {table}_v8 SELECT * FROM {table};"
            ))?;
            tables.push(table);
        }
        for table in tables.iter().rev() {
            tx.execute_batch(&format!("DROP TABLE {table};"))?;
        }
        for table in tables {
            tx.execute_batch(&format!("ALTER TABLE {table}_v8 RENAME TO {table};"))?;
        }
        for sql in objects {
            tx.execute_batch(&sql)?;
        }
        let violation: Option<String> = tx
            .query_row("PRAGMA foreign_key_check", [], |r| r.get(0))
            .optional()?;
        ensure!(
            violation.is_none(),
            "Invalid library references: {violation:?}"
        );
        // Older versions could already have left a deleted selection in JSON.
        // It has no FK and must be cleared before the first post-upgrade import.
        super::super::save_session(&tx, &super::super::load_session(&tx)?)?;
        tx.pragma_update(None, "user_version", 8)?;
        tx.commit()?;
        Ok(())
    })();
    let restored_alter = conn.pragma_update(None, "legacy_alter_table", legacy_alter);
    let restored_keys = conn.pragma_update(None, "foreign_keys", true);
    result?;
    restored_alter?;
    restored_keys?;
    Ok(())
}
