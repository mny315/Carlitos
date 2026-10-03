use super::*;
use std::path::Path;

fn legacy(path: &Path) -> Result<Store> {
    let conn = Connection::open(path)?;
    conn.execute_batch(include_str!("../schema.sql"))?;
    conn.execute_batch(
        "DROP TABLE queue_entries;
         ALTER TABLE media_files DROP COLUMN music;
         CREATE TABLE source_files(
             source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
             file_id INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
             relative TEXT NOT NULL, PRIMARY KEY(source_id,file_id), UNIQUE(source_id,relative));
         CREATE INDEX book_parts_file ON book_parts(file_id);
         CREATE INDEX chapters_part ON chapters(part_id);
         CREATE INDEX source_files_file ON source_files(file_id);
         PRAGMA user_version=4;",
    )?;
    let mut store = Store { conn };
    let mut book = draft("legacy", 2);
    book.files[0].chapters.push(Chapter {
        title: "Saved chapter".into(),
        start: 123,
        end: Some(345),
    });
    let mut library = store.import(vec![book])?;
    library.session.current = Some(Target::Book(library.parts[0].id));
    library.update_progress(12_345, false);
    library.session.volume = 0.37;
    library.session.muted = true;
    store.save(&library.session, &library.progress)?;
    store.edit_book(
        library.books[0].id,
        "Manual title".into(),
        "Manual author".into(),
        library.parts.iter().rev().map(|part| part.id).collect(),
        Some("/custom/cover.png".into()),
    )?;
    Ok(store)
}

fn snapshot(library: Library) -> serde_json::Value {
    serde_json::json!({
        "sources": library.sources, "links": library.source_files, "media": library.media,
        "books": library.books, "parts": library.parts, "progress": library.progress,
        "session": library.session,
    })
}

#[test]
fn v4_path_migration_preserves_all_library_data_indexes_and_triggers() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("db");
    let old = legacy(&path)?;
    old.conn.execute_batch(
        "CREATE TABLE audit(file_id INTEGER);
         CREATE INDEX extra_media_modified ON media_files(modified);
         CREATE INDEX extra_source_relative ON source_files(relative);
         CREATE TRIGGER extra_media_update AFTER UPDATE ON media_files
         BEGIN INSERT INTO audit VALUES(new.id); END;",
    )?;
    let before = snapshot(old.load()?);
    drop(old);
    let mut migrated = Store::open(&path)?;
    assert_eq!(snapshot(migrated.load()?), before);
    assert_eq!(
        migrated
            .conn
            .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
        5
    );
    assert_eq!(
        migrated
            .conn
            .pragma_query_value(None, "foreign_keys", |r| r.get::<_, i64>(0))?,
        1
    );
    assert!(
        !migrated
            .conn
            .prepare("PRAGMA foreign_key_check")?
            .exists([])?
    );
    for name in [
        "book_parts_file",
        "chapters_part",
        "source_files_file",
        "media_files_source_path",
        "extra_media_modified",
        "extra_source_relative",
        "extra_media_update",
    ] {
        assert!(
            migrated
                .conn
                .prepare("SELECT 1 FROM sqlite_schema WHERE name=?1")?
                .exists([name])?,
            "{name}"
        );
    }
    assert!(!migrated.conn.prepare("SELECT 1 FROM audit")?.exists([])?);
    migrated
        .conn
        .execute("UPDATE media_files SET modified=modified WHERE id=1", [])?;
    assert!(
        migrated
            .conn
            .prepare("SELECT 1 FROM audit WHERE file_id=1")?
            .exists([])?
    );
    assert!(migrated.conn.execute("DELETE FROM sources", []).is_err());
    let library = migrated.load()?;
    migrated.remove_source(library.sources[0].id)?;
    assert!(migrated.load()?.parts.is_empty());
    assert!(
        !migrated
            .conn
            .prepare("SELECT 1 FROM chapters")?
            .exists([])?
    );
    assert!(
        !migrated
            .conn
            .prepare("PRAGMA foreign_key_check")?
            .exists([])?
    );
    Ok(())
}

#[test]
fn invalid_v4_references_roll_back_the_entire_path_migration() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("db");
    let old = legacy(&path)?;
    old.conn.pragma_update(None, "foreign_keys", false)?;
    old.conn
        .execute("UPDATE book_parts SET file_id=999 WHERE id=1", [])?;
    drop(old);
    assert!(Store::open(&path).is_err());
    let conn = Connection::open(&path)?;
    assert_eq!(
        conn.pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
        4
    );
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM chapters", [], |r| r.get::<_, i64>(0))?,
        1
    );
    assert_eq!(
        conn.query_row("SELECT position FROM book_progress", [], |r| r
            .get::<_, i64>(0))?,
        12_345
    );
    assert!(
        !conn
            .prepare(
                "SELECT 1 FROM sqlite_schema WHERE name IN ('media_files_v5','source_files_v5')"
            )?
            .exists([])?
    );
    // The original UNIQUE constraint is restored by rollback too.
    assert!(conn.execute("INSERT INTO media_files SELECT 99,source_id,'file:///other',relative,'other',size,modified,data FROM media_files WHERE id=1", []).is_err());
    conn.execute("UPDATE book_parts SET file_id=1 WHERE id=1", [])?;
    drop(conn);
    assert_eq!(Store::open(&path)?.load()?.progress[0].position, 12_345);
    Ok(())
}

#[test]
fn simultaneous_opens_migrate_paths_once_and_keep_the_checkpoint() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("db");
    let old = legacy(&path)?;
    let before = snapshot(old.load()?);
    drop(old);
    let barrier = std::sync::Barrier::new(4);
    std::thread::scope(|scope| -> Result<()> {
        let threads: Vec<_> = (0..4)
            .map(|_| {
                scope.spawn(|| -> Result<_> {
                    barrier.wait();
                    Ok(snapshot(Store::open(&path)?.load()?))
                })
            })
            .collect();
        for thread in threads {
            assert_eq!(thread.join().unwrap()?, before);
        }
        Ok(())
    })
}
