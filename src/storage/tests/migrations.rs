use super::*;

#[test]
fn concurrent_opens_initialize_and_migrate_legacy_schemas_once() -> Result<()> {
    for version in 0..=3 {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("db");
        if version > 0 {
            let conn = Connection::open(&path)?;
            conn.execute_batch(include_str!("../schema.sql"))?;
            if version >= 2 {
                conn.execute_batch(
                    "CREATE TABLE source_files(
                        source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
                        file_id INTEGER NOT NULL REFERENCES media_files(id) ON DELETE CASCADE,
                        relative TEXT NOT NULL, PRIMARY KEY(source_id,file_id), UNIQUE(source_id,relative));",
                )?;
            }
            if version >= 3 {
                conn.execute_batch(
                    "DROP TABLE queue_entries; ALTER TABLE media_files DROP COLUMN music;",
                )?;
            }
            conn.pragma_update(None, "user_version", version)?;
        }
        let barrier = std::sync::Barrier::new(8);
        std::thread::scope(|scope| -> Result<()> {
            let threads: Vec<_> = (0..8)
                .map(|_| {
                    scope.spawn(|| -> Result<()> {
                        barrier.wait();
                        let store = Store::open(&path)?;
                        assert!(store.load()?.books.is_empty());
                        assert_eq!(
                            store
                                .conn
                                .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))?,
                            5
                        );
                        Ok(())
                    })
                })
                .collect();
            for thread in threads {
                thread
                    .join()
                    .unwrap()
                    .with_context(|| format!("Concurrent migration from schema {version}"))?;
            }
            Ok(())
        })?;
    }
    Ok(())
}

#[test]
fn migration_from_v1_and_shared_source_removal() -> Result<()> {
    let t = tempfile::tempdir()?;
    let path = t.path().join("db");
    let c = Connection::open(&path)?;
    c.execute_batch(include_str!("../schema.sql"))?;
    c.pragma_update(None, "user_version", 1)?;
    drop(c);
    let mut store = Store::open(&path)?;
    let book = draft("shared", 2);
    let lib = store.import(vec![book])?;
    assert_eq!(lib.media.len(), 2);
    assert_eq!(lib.books.len(), 1);
    let lib = store.remove_source(lib.sources[0].id)?;
    assert!(lib.media.is_empty());
    assert!(lib.books.is_empty());
    assert!(lib.sources.is_empty());
    Ok(())
}

#[test]
fn migration_removes_music_session_but_preserves_shared_books() -> Result<()> {
    for current in [
        serde_json::json!({"Music": 7}),
        serde_json::json!({"Book": 12}),
    ] {
        let temp = tempfile::tempdir()?;
        let path = temp.path().join("db");
        let conn = Connection::open(&path)?;
        conn.execute_batch(include_str!("../schema.sql"))?;
        conn.execute_batch("CREATE TABLE source_files(source_id INTEGER REFERENCES sources(id),file_id INTEGER REFERENCES media_files(id),relative TEXT,PRIMARY KEY(source_id,file_id));
            INSERT INTO sources VALUES(1,'file:///shared','music'),(2,'file:///shared','book');
            INSERT INTO books VALUES(10,2,'Saved title','Saved author',NULL);")?;
        let media = draft("shared", 2).files;
        for (i, file) in media.iter().enumerate() {
            conn.execute(
                "INSERT INTO media_files VALUES(?1,1,?2,?3,?4,0,0,?5,1)",
                params![
                    i as i64 + 1,
                    file.uri,
                    file.relative,
                    file.identity,
                    serde_json::to_string(file)?
                ],
            )?;
            conn.execute(
                "INSERT INTO source_files VALUES(1,?1,?2)",
                params![i as i64 + 1, file.relative],
            )?;
        }
        conn.execute_batch(
            "INSERT INTO source_files VALUES(2,1,'0.wav');
            INSERT INTO book_parts VALUES(12,10,1,0,'Custom part');
            INSERT INTO book_progress VALUES(10,12,32000,0,123);
            INSERT INTO queue_entries VALUES(7,1,0);",
        )?;
        let state = serde_json::json!({"current": current,"position":32000,"volume":0.4,"muted":true,"shuffle":true,"repeat":"All","shuffle_order":[7],"next_queue_id":8});
        conn.execute(
            "INSERT INTO session_state VALUES(1,?1)",
            [state.to_string()],
        )?;
        conn.pragma_update(None, "user_version", 2)?;
        drop(conn);
        let store = Store::open(&path)?;
        let lib = store.load()?;
        assert_eq!(lib.books[0].title, "Saved title");
        assert_eq!(lib.parts[0].title, "Custom part");
        assert_eq!(lib.progress[0].part_id, 12);
        assert_eq!(lib.progress[0].position, 32000);
        assert_eq!(lib.media.len(), 1);
        assert_eq!(lib.sources.len(), 1);
        assert_eq!(lib.session.volume, 0.4);
        assert!(lib.session.muted);
        if current.get("Music").is_some() {
            assert_eq!(lib.session.current, None);
            assert_eq!(lib.session.position, 0);
        } else {
            assert_eq!(lib.session.current, Some(Target::Book(12)));
            assert_eq!(lib.session.position, 32000);
        }
        assert_eq!(
            store
                .conn
                .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
            5
        );
        assert!(store.conn.prepare("SELECT * FROM queue_entries").is_err());
        assert!(store.conn.prepare("SELECT music FROM media_files").is_err());
        let saved: String = store
            .conn
            .query_row("SELECT data FROM session_state", [], |r| r.get(0))?;
        assert!(!saved.contains("shuffle") && !saved.contains("repeat"));
        drop(store);
        assert_eq!(Store::open(&path)?.load()?.progress[0].position, 32000);
    }
    Ok(())
}

#[test]
fn migration_from_v3_preserves_books_and_indexes_reimport_lookups() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("db");
    let mut store = Store::open(&path)?;
    let mut original = draft("indexed", 32);
    original.files[0].chapters.push(Chapter {
        title: "Chapter".into(),
        start: 0,
        end: Some(60_000),
    });
    let mut library = store.import(vec![original.clone()])?;
    library.session.current = Some(Target::Book(library.parts[0].id));
    library.update_progress(12_000, false);
    store.save(&library.session, &library.progress)?;
    let order: Vec<_> = library.parts.iter().rev().map(|p| p.id).collect();
    store.edit_book(
        library.books[0].id,
        "Manual title".into(),
        "Manual author".into(),
        order.clone(),
        None,
    )?;
    store.conn.execute_batch(
        "DROP INDEX book_parts_file; DROP INDEX chapters_part;
         DROP INDEX source_files_file; PRAGMA user_version=3;",
    )?;
    drop(store);

    let mut store = Store::open(&path)?;
    let reimported = store.import(vec![original])?;
    assert_eq!(reimported.books[0].title, "Manual title");
    assert_eq!(reimported.books[0].author, "Manual author");
    assert_eq!(
        reimported.parts.iter().map(|p| p.id).collect::<Vec<_>>(),
        order
    );
    assert_eq!(reimported.session.current, library.session.current);
    assert_eq!(reimported.progress[0].position, 12_000);
    assert_eq!(reimported.media[0].chapters[0].title, "Chapter");
    for query in [
        "SELECT book_id FROM book_parts WHERE file_id=?1",
        "SELECT id FROM chapters WHERE part_id=?1",
        "SELECT source_id FROM source_files WHERE file_id=?1",
    ] {
        let plan: String = store.conn.query_row(
            &format!("EXPLAIN QUERY PLAN {query}"),
            [library.parts[0].file_id],
            |row| row.get(3),
        )?;
        assert!(plan.starts_with("SEARCH "), "{query}: {plan}");
    }
    drop(store);
    assert_eq!(Store::open(&path)?.load()?.progress[0].position, 12_000);
    Ok(())
}

#[test]
fn invalid_legacy_session_rolls_back_migration() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("db");
    let conn = Connection::open(&path)?;
    conn.execute_batch(include_str!("../schema.sql"))?;
    conn.execute_batch(
        "INSERT INTO session_state VALUES(1,'invalid json'); PRAGMA user_version=2;",
    )?;
    drop(conn);
    assert!(Store::open(&path).is_err());
    let conn = Connection::open(&path)?;
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        2
    );
    conn.prepare("SELECT * FROM queue_entries")?;
    conn.prepare("SELECT music FROM media_files")?;
    Ok(())
}

#[test]
fn migration_dedup_progress_and_manual_order_survive_reopen() -> Result<()> {
    let tmp = tempfile::tempdir()?;
    let path = tmp.path().join("db");
    let mut s = Store::open(&path)?;
    let l = s.import(vec![draft("книга с пробелом", 3), draft("other", 2)])?;
    assert_eq!(l.books.len(), 2);
    let mut ids: Vec<_> = l.book_parts(l.books[0].id).iter().map(|p| p.id).collect();
    ids.reverse();
    s.edit_book(
        l.books[0].id,
        "Manual".into(),
        "Author".into(),
        ids.clone(),
        None,
    )?;
    let mut l = s.load()?;
    l.session.current = Some(Target::Book(ids[0]));
    l.update_progress(32_000, false);
    s.save(&l.session, &l.progress)?;
    s.import(vec![draft("книга с пробелом", 3), draft("other", 2)])?;
    drop(s);
    let l = Store::open(&path)?.load()?;
    assert_eq!(l.media.len(), 5);
    assert_eq!(l.books.len(), 2);
    assert_eq!(l.progress[0].position, 32_000);
    assert_eq!(l.book_parts(l.progress[0].book_id)[0].id, ids[0]);
    assert_eq!(l.session.position, 32_000);
    Ok(())
}

#[test]
fn future_schema_is_not_modified() -> Result<()> {
    let t = tempfile::tempdir()?;
    let p = t.path().join("db");
    let c = Connection::open(&p)?;
    c.pragma_update(None, "user_version", 99)?;
    drop(c);
    assert!(Store::open(&p).is_err());
    assert_eq!(
        Connection::open(&p)?.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?,
        99
    );
    Ok(())
}
