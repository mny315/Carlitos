use super::*;
use std::path::Path;

fn version7(path: &Path) -> Result<Store> {
    let mut store = super::path_migration::legacy(path)?;
    store.conn.pragma_update(None, "foreign_keys", false)?;
    store
        .conn
        .execute_batch(include_str!("../migrations/document_paths.sql"))?;
    store.conn.execute_batch(
        "CREATE INDEX source_files_file ON source_files(file_id);
         CREATE INDEX media_files_source_path ON media_files(source_id,relative);
         PRAGMA user_version=5;",
    )?;
    store.conn.pragma_update(None, "foreign_keys", true)?;
    super::super::sync::migrate(&mut store.conn)?;
    super::super::sync::migrate_rates(&mut store.conn)?;
    Ok(store)
}

#[test]
fn stable_id_migration_preserves_library_sync_and_custom_schema_objects() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("db");
    let mut old = version7(&database)?;
    let before = old.load()?;
    let book = before.books[0].id;
    old.save_book_rate(book, 1.75)?;
    old.save_fingerprints(
        &before,
        &before
            .media
            .iter()
            .map(|file| crate::sync::Fingerprint {
                file: file.id,
                signature: crate::sync::Fingerprint::signature(file),
                hash: "ab".repeat(32),
            })
            .collect::<Vec<_>>(),
    )?;
    let device = old.sync_device()?;
    let records = old.sync_records(&before)?;
    let rates = old.sync_rates(&before)?;
    let fingerprints = old.fingerprints()?;
    old.conn.execute_batch(
        "CREATE TABLE audit(part_id INTEGER);
         CREATE INDEX extra_book_title ON books(title);
         CREATE TRIGGER extra_part_update AFTER UPDATE ON book_parts
         BEGIN INSERT INTO audit VALUES(NEW.id); END;
         CREATE VIEW extra_books AS SELECT b.id,p.file_id FROM books b JOIN book_parts p ON p.book_id=b.id;
         CREATE TRIGGER extra_audit_insert AFTER INSERT ON audit
         BEGIN SELECT COUNT(*) FROM books; END;",
    )?;
    let schema = |conn: &Connection| -> Result<Vec<(String, String)>> {
        Ok(conn
            .prepare("SELECT name,sql FROM sqlite_schema WHERE name LIKE 'extra_%' ORDER BY name")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?)
    };
    let objects = schema(&old.conn)?;
    drop(old);
    let mut migrated = Store::open(&database)?;
    let after = migrated.load()?;
    assert_eq!(
        super::path_migration::snapshot(after.clone()),
        super::path_migration::snapshot(before)
    );
    assert_eq!(migrated.sync_device()?, device);
    assert_eq!(migrated.sync_records(&after)?, records);
    assert_eq!(migrated.sync_rates(&after)?, rates);
    assert_eq!(
        serde_json::to_value(migrated.fingerprints()?)?,
        serde_json::to_value(fingerprints)?
    );
    assert_eq!(schema(&migrated.conn)?, objects);
    assert_eq!(
        migrated
            .conn
            .query_row("SELECT COUNT(*) FROM audit", [], |r| r.get::<_, i64>(0))?,
        0
    );
    assert_eq!(
        migrated
            .conn
            .query_row("SELECT COUNT(*) FROM extra_books", [], |r| r
                .get::<_, i64>(0))?,
        after.parts.len() as i64
    );
    for table in ["sources", "media_files", "books", "book_parts"] {
        let sql: String = migrated.conn.query_row(
            "SELECT sql FROM sqlite_schema WHERE type='table' AND name=?1",
            [table],
            |r| r.get(0),
        )?;
        assert!(sql.contains("AUTOINCREMENT"), "{table}: {sql}");
    }
    assert!(
        !migrated
            .conn
            .prepare("PRAGMA foreign_key_check")?
            .exists([])?
    );
    migrated.conn.execute(
        "UPDATE book_parts SET title='Changed' WHERE id=?1",
        [after.parts[0].id],
    )?;
    assert_eq!(
        migrated
            .conn
            .query_row("SELECT COUNT(*) FROM audit", [], |r| r.get::<_, i64>(0))?,
        1
    );
    migrated.remove_book(book)?;
    assert!(migrated.load()?.parts.is_empty());
    assert!(migrated.load()?.progress.is_empty());
    Ok(())
}

#[test]
fn failed_stable_id_migration_rolls_back_all_tables_and_revisions() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("db");
    let old = version7(&database)?;
    old.conn.pragma_update(None, "foreign_keys", false)?;
    old.conn
        .execute("UPDATE book_parts SET file_id=999 WHERE id=1", [])?;
    drop(old);
    assert!(Store::open(&database).is_err());
    let conn = Connection::open(&database)?;
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
        7
    );
    for table in ["sources", "media_files", "books", "book_parts"] {
        let sql: String = conn.query_row(
            "SELECT sql FROM sqlite_schema WHERE name=?1",
            [table],
            |r| r.get(0),
        )?;
        assert!(!sql.contains("AUTOINCREMENT"));
    }
    assert!(
        !conn
            .prepare("SELECT 1 FROM sqlite_schema WHERE name LIKE '%_v8'")?
            .exists([])?
    );
    assert_eq!(
        conn.query_row("SELECT position FROM book_progress", [], |r| r
            .get::<_, i64>(0))?,
        12_345
    );
    conn.execute("UPDATE book_parts SET file_id=1 WHERE id=1", [])?;
    drop(conn);
    assert_eq!(Store::open(&database)?.load()?.progress[0].position, 12_345);
    Ok(())
}

#[test]
fn library_ids_are_not_reused_after_deletion_or_restart() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("db");
    let mut previous = [0; 4];
    for name in ["first", "second", "third"] {
        let mut store = Store::open(&database)?;
        let mut before = store.import(vec![draft(name, 1)])?;
        let ids = [
            before.sources[0].id,
            before.media[0].id,
            before.books[0].id,
            before.parts[0].id,
        ];
        assert!(
            ids.iter()
                .zip(previous)
                .all(|(id, previous)| *id > previous)
        );
        previous = ids;
        before.session.current = Some(Target::Book(before.parts[0].id));
        before.update_progress(12_000, false);
        store.remove_source(before.sources[0].id)?;
        let after = store.import(vec![draft(&format!("{name}-new"), 1)])?;
        // A delayed checkpoint must not become the new book's progress either.
        store.save(&before.session, &before.progress)?;
        let saved = store.load()?;
        assert!(saved.session.current.is_none());
        assert!(saved.progress.is_empty());
        store.remove_source(after.sources[0].id)?;
    }
    Ok(())
}

#[test]
fn concurrent_opens_migrate_stable_ids_once_without_changing_progress() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("db");
    let old = version7(&database)?;
    let expected = super::path_migration::snapshot(old.load()?);
    drop(old);
    let start = std::sync::Barrier::new(6);
    std::thread::scope(|scope| -> Result<()> {
        let threads: Vec<_> = (0..6)
            .map(|_| {
                scope.spawn(|| -> Result<()> {
                    start.wait();
                    let store = Store::open(&database)?;
                    assert_eq!(super::path_migration::snapshot(store.load()?), expected);
                    assert_eq!(
                        store
                            .conn
                            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
                        8
                    );
                    Ok(())
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap()?;
        }
        Ok(())
    })
}

#[test]
fn migration_clears_an_old_dangling_selection_before_an_import_can_reuse_its_number() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("db");
    let old = version7(&database)?;
    // Simulate legacy removal which did not validate the JSON session target.
    old.conn.execute("DELETE FROM books", [])?;
    drop(old);
    let mut store = Store::open(&database)?;
    let library = store.import(vec![draft("new", 1)])?;
    assert!(library.session.current.is_none());
    assert_eq!(library.session.position, 0);
    assert!(library.progress.is_empty());
    Ok(())
}
