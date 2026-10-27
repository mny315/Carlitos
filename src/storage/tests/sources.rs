use super::*;

#[test]
fn removing_multiple_sources_preserves_unselected_books_progress_and_files() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let mut drafts = vec![];
    let mut paths = vec![];
    for name in ["one", "two", "keep"] {
        let root = temp.path().join(name);
        std::fs::create_dir(&root)?;
        let path = root.join("0.wav");
        std::fs::write(&path, b"original audio")?;
        let mut d = draft(name, 1);
        d.root_uri = file_uri(&root)?;
        d.files[0].uri = file_uri(&path)?;
        paths.push(path);
        drafts.push(d);
    }
    let mut before = store.import(drafts)?;
    let keep = before.books.iter().find(|b| b.title == "keep").unwrap();
    let keep_source = keep.source_id;
    let keep_book = keep.id;
    let keep_part = before.book_parts(keep_book)[0].id;
    for book in before.books.clone() {
        before.session.current = Some(Target::Book(before.book_parts(book.id)[0].id));
        before.update_progress(12_000, false);
    }
    before.session.current = Some(Target::Book(keep_part));
    store.save(&before.session, &before.progress)?;
    let mut ids: Vec<_> = before
        .sources
        .iter()
        .filter(|s| s.id != keep_source)
        .map(|s| s.id)
        .collect();
    ids.extend([ids[0], i64::MAX]); // Duplicate and already removed IDs are harmless.
    let after = store.remove_sources(&ids)?;
    assert_eq!(after.sources.len(), 1);
    assert_eq!(after.sources[0].id, keep_source);
    assert_eq!(after.books.len(), 1);
    assert_eq!(after.books[0].id, keep_book);
    assert_eq!(after.parts.len(), 1);
    assert_eq!(after.parts[0].id, keep_part);
    assert_eq!(after.progress.len(), 1);
    assert_eq!(after.progress[0].position, 12_000);
    assert_eq!(after.session.current, Some(Target::Book(keep_part)));
    assert!(
        paths
            .iter()
            .all(|p| std::fs::read(p).unwrap() == b"original audio")
    );
    let empty = store.remove_sources(&[keep_source])?;
    assert!(empty.sources.is_empty() && empty.books.is_empty() && empty.media.is_empty());
    assert!(empty.session.current.is_none());
    assert!(paths.iter().all(|p| p.is_file()));
    Ok(())
}

#[test]
fn multiple_source_removal_rolls_back_the_whole_selection_on_failure() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let mut before = store.import(vec![draft("one", 1), draft("two", 1)])?;
    before.session.current = Some(Target::Book(before.parts[0].id));
    before.update_progress(8_000, false);
    store.save(&before.session, &before.progress)?;
    let ids: Vec<_> = before.sources.iter().map(|s| s.id).collect();
    store.conn.execute_batch(&format!(
        "CREATE TRIGGER fail_second_source BEFORE DELETE ON sources WHEN OLD.id={} BEGIN SELECT RAISE(ABORT, 'test batch failure'); END;", ids[1]
    ))?;
    assert!(store.remove_sources(&ids).is_err());
    let after = store.load()?;
    assert_eq!(after.sources.iter().map(|s| s.id).collect::<Vec<_>>(), ids);
    assert_eq!(after.books.len(), before.books.len());
    assert_eq!(
        after.parts.iter().map(|p| p.id).collect::<Vec<_>>(),
        before.parts.iter().map(|p| p.id).collect::<Vec<_>>()
    );
    assert_eq!(after.session.current, before.session.current);
    assert_eq!(after.progress[0].position, 8_000);
    Ok(())
}

#[test]
fn multiple_overlapping_sources_keep_surviving_document_access() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let mut drafts = vec![];
    for tree in ["one", "two", "keep"] {
        let root = format!("content://provider/tree/{tree}/document/{tree}");
        let mut d = draft(tree, 1);
        d.root_uri = root;
        d.files[0].uri = format!("content://provider/tree/{tree}/document/shared");
        d.files[0].identity = crate::source::document_identity(&d.files[0].uri).unwrap();
        drafts.push(d);
    }
    let before = store.import(drafts)?;
    let ids: Vec<_> = before.sources.iter().map(|s| s.id).collect();
    let after = store.remove_sources(&ids[..2])?;
    assert_eq!(after.sources.len(), 1);
    assert_eq!(after.sources[0].id, ids[2]);
    assert_eq!(after.media.len(), 1);
    assert_eq!(after.media[0].source_id, ids[2]);
    assert_eq!(
        after.media[0].uri,
        "content://provider/tree/keep/document/shared"
    );
    Ok(())
}

#[test]
fn library_reads_one_snapshot_while_a_source_is_relocated() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("db");
    let mut store = Store::open(&database)?;
    let mut imported = draft("snapshot", 32);
    if cfg!(windows) {
        imported.root_uri = imported.root_uri.replacen("file:///", "file:///C:/", 1);
        for file in &mut imported.files {
            file.uri = file.uri.replacen("file:///", "file:///C:/", 1);
        }
    }
    store.import(vec![imported])?;
    let start = std::sync::Barrier::new(2);
    std::thread::scope(|scope| -> Result<()> {
        let writer = scope.spawn(|| -> Result<()> {
            let mut conn = Connection::open(&database)?;
            conn.busy_timeout(std::time::Duration::from_secs(5))?;
            start.wait();
            for i in 0..200 {
                let root = format!(
                    "file:///{}snapshot-{i}",
                    if cfg!(windows) { "C:/" } else { "" }
                );
                let tx = conn.transaction()?;
                tx.execute("UPDATE sources SET uri=?1", [&root])?;
                tx.execute("UPDATE media_files SET uri=?1 || '/' || relative", [&root])?;
                tx.commit()?;
                std::thread::yield_now();
            }
            Ok(())
        });
        start.wait();
        let mut mixed = false;
        for _ in 0..200 {
            let library = store.load()?;
            let root = local_path(&library.sources[0].uri).unwrap();
            mixed |= library
                .media
                .iter()
                .any(|file| local_path(&file.uri).as_ref() != Some(&root.join(&file.relative)));
        }
        writer.join().unwrap()?;
        assert!(
            !mixed,
            "a library must not combine rows from different commits"
        );
        Ok(())
    })
}

#[test]
fn removing_overlapping_source_keeps_other_books_and_shared_parts() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let inner = draft("outer/inner", 1);
    let mut library = store.import(vec![inner.clone()])?;
    let book = library.books[0].id;
    let part = library.parts[0].id;
    library.session.current = Some(Target::Book(part));
    library.update_progress(12_000, false);
    store.save(&library.session, &library.progress)?;
    let mut outer = draft("outer", 1);
    let mut shared = inner.files[0].clone();
    shared.relative = "inner/0.wav".into();
    outer.files.push(shared);
    let library = store.import(vec![outer])?;
    assert_eq!(library.book_parts(book).len(), 2);
    let source = library
        .sources
        .iter()
        .find(|s| s.uri == "file:///outer")
        .unwrap()
        .id;
    let after = store.remove_source(source)?;
    assert_eq!(after.books.len(), 1);
    assert_eq!(after.parts.len(), 1);
    assert_eq!(after.parts[0].id, part);
    assert_eq!(after.progress[0].position, 12_000);
    assert_eq!(after.media.len(), 1);
    assert_eq!(after.sources.len(), 1);
    Ok(())
}

#[test]
fn rescan_can_reuse_an_inode_from_an_unavailable_record() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let old = draft("unavailable", 1);
    let mut updated = draft("replaced", 1);
    let before = store.import(vec![old.clone(), updated.clone()])?;
    updated.files[0].identity = old.files[0].identity.clone();
    updated.files[0].duration = Some(72_000);
    let after = store.import(vec![updated])?;
    assert_eq!(
        after.parts.iter().map(|p| p.id).collect::<Vec<_>>(),
        before.parts.iter().map(|p| p.id).collect::<Vec<_>>()
    );
    assert_eq!(after.media[1].duration, Some(72_000));
    assert_eq!(after.media[0].duration, Some(60_000));
    Ok(())
}

#[test]
fn stale_inode_cannot_merge_an_unrelated_book() -> Result<()> {
    let t = tempfile::tempdir()?;
    let mut store = Store::open(&t.path().join("db"))?;
    let first = draft("missing_old_recording", 1);
    let mut lib = store.import(vec![first.clone()])?;
    lib.session.current = Some(Target::Book(lib.parts[0].id));
    lib.update_progress(5_000, false);
    store.save(&lib.session, &lib.progress)?;
    let mut second = draft("new_recording", 1);
    second.files[0].identity = first.files[0].identity.clone();
    second.files[0].title = "Different recording".into();
    let after = store.import(vec![second])?;
    assert_eq!(after.media.len(), 2);
    assert_eq!(after.books.len(), 2);
    assert_eq!(after.progress[0].part_id, lib.parts[0].id);
    assert_eq!(after.progress[0].position, 5_000);
    assert_eq!(after.media[0].title, first.files[0].title);
    Ok(())
}

#[test]
fn real_hardlinks_share_a_stable_file_and_part() -> Result<()> {
    let t = tempfile::tempdir()?;
    let one = t.path().join("one");
    let two = t.path().join("two");
    std::fs::create_dir(&one)?;
    std::fs::create_dir(&two)?;
    std::fs::write(one.join("0.wav"), b"fixture")?;
    std::fs::hard_link(one.join("0.wav"), two.join("0.wav"))?;
    let mut store = Store::open(&t.path().join("db"))?;
    let mut drafts = vec![];
    for root in [one, two] {
        let mut d = draft("hardlink", 1);
        d.root_uri = file_uri(&root)?;
        d.files[0].uri = file_uri(&root.join("0.wav"))?;
        d.files[0].identity = crate::import::identity(&root.join("0.wav"))?;
        drafts.push(d);
    }
    let lib = store.import(drafts)?;
    assert_eq!(lib.sources.len(), 2);
    assert_eq!(lib.media.len(), 1);
    assert_eq!(lib.books.len(), 1);
    assert_eq!(lib.parts.len(), 1);
    Ok(())
}

#[test]
fn removing_books_cleans_empty_book_sources_and_repairs_old_removals() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("db");
    let mut store = Store::open(&database)?;
    let before = store.import(vec![draft("one", 1), draft("keep", 1)])?;
    let first = before.books.iter().find(|b| b.title == "one").unwrap();
    let other = before.books.iter().find(|b| b.title == "keep").unwrap();
    let collection = store.ensure_folder("file:///collection")?;
    let alias = store.conn.query_row(
        "INSERT INTO sources(uri,kind) VALUES('content://provider/document/alias','book') RETURNING id", [], |r| r.get::<_, Id>(0),
    )?;
    store.conn.execute(
        "INSERT INTO source_files(source_id,file_id,relative) VALUES(?1,?2,'alias.wav')",
        params![alias, before.book_parts(other.id)[0].file_id],
    )?;
    let after = store.remove_books(&[first.id])?;
    assert!(!after.sources.iter().any(|s| s.id == first.source_id));
    assert_eq!(after.books.len(), 1);
    assert_eq!(after.books[0].id, other.id);
    assert!(after.sources.iter().any(|s| s.id == collection));
    assert!(after.sources.iter().any(|s| s.id == alias));
    assert_eq!(after.media.len(), 1);
    assert_eq!(after.parts[0].id, before.book_parts(other.id)[0].id);
    // Simulate the old code leaving book/source/file records behind.
    store
        .conn
        .execute("DELETE FROM books WHERE id=?1", [other.id])?;
    drop(store);
    let after = Store::open(&database)?.load()?;
    assert!(after.books.is_empty() && after.parts.is_empty() && after.media.is_empty());
    assert_eq!(after.sources.len(), 1);
    assert_eq!(after.sources[0].id, collection);
    Ok(())
}
