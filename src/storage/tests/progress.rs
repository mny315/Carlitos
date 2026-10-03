use super::*;

#[test]
fn checkpoints_write_only_changed_progress_and_session() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let mut library = store.import(vec![draft("one", 1), draft("two", 1)])?;
    for part in library.parts.clone() {
        library.session.current = Some(Target::Book(part.id));
        library.update_progress(10_000, false);
    }
    store.save(&library.session, &library.progress)?;
    let saved = store.conn.total_changes();
    store.save(&library.session, &library.progress)?;
    assert_eq!(store.conn.total_changes(), saved);

    library.update_progress(14_000, false);
    store.save(&library.session, &library.progress)?;
    assert_eq!(store.conn.total_changes() - saved, 2);
    let saved = store.conn.total_changes();
    library.progress[0].completed = true;
    library.progress[0].updated += 1;
    store.save(&library.session, &library.progress)?;
    assert_eq!(store.conn.total_changes() - saved, 1);
    let restored = store.load()?;
    assert_eq!(restored.session.position, 14_000);
    assert_eq!(restored.progress[0].position, 10_000);
    assert!(restored.progress[0].completed);
    assert_eq!(restored.progress[0].updated, library.progress[0].updated);
    assert_eq!(restored.progress[1].position, 14_000);
    Ok(())
}

#[test]
fn removed_selection_cannot_attach_to_a_reused_part_id() -> Result<()> {
    for remove_source in [false, true] {
        let temp = tempfile::tempdir()?;
        let mut store = Store::open(&temp.path().join("db"))?;
        let mut library = store.import(vec![draft("old", 1)])?;
        let part = library.parts[0].id;
        library.session.current = Some(Target::Book(part));
        library.update_progress(12_000, false);
        store.save(&library.session, &library.progress)?;
        if remove_source {
            store.remove_source(library.sources[0].id)?;
        } else {
            store.remove_book(library.books[0].id)?;
        }
        let after = store.import(vec![draft("new", 1)])?;
        assert_eq!(after.parts[0].id, part, "fixture must exercise ID reuse");
        assert!(after.session.current.is_none());
        assert_eq!(after.session.position, 0);
        assert!(after.progress.is_empty());
    }
    Ok(())
}

#[test]
fn stale_checkpoint_cannot_leave_a_selection_for_a_future_import() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let mut library = store.import(vec![draft("old", 1)])?;
    library.session.current = Some(Target::Book(library.parts[0].id));
    library.update_progress(12_000, false);
    store.remove_source(library.sources[0].id)?;
    store.save(&library.session, &library.progress)?;
    let after = store.import(vec![draft("new", 1)])?;
    assert!(after.session.current.is_none());
    assert_eq!(after.session.position, 0);
    assert!(after.progress.is_empty());
    Ok(())
}

#[test]
fn stale_checkpoint_for_a_removed_part_keeps_the_surviving_book() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let inner = draft("outer/inner", 1);
    let library = store.import(vec![inner.clone()])?;
    let book = library.books[0].id;
    let mut outer = draft("outer", 1);
    let mut shared = inner.files[0].clone();
    shared.relative = "inner/0.wav".into();
    outer.files.push(shared);
    let mut library = store.import(vec![outer])?;
    let source = library
        .sources
        .iter()
        .find(|s| s.uri == "file:///outer")
        .unwrap()
        .id;
    let exclusive = library
        .parts
        .iter()
        .find(|p| library.media(p.file_id).unwrap().source_id == source)
        .unwrap()
        .id;
    library.session.current = Some(Target::Book(exclusive));
    library.update_progress(12_000, false);
    store.save(&library.session, &library.progress)?;
    store.remove_source(source)?;
    store.save(&library.session, &library.progress)?;
    let after = store.load()?;
    assert_eq!(after.books[0].id, book);
    assert_eq!(after.parts.len(), 1);
    assert!(after.session.current.is_none());
    assert!(after.progress.is_empty());
    Ok(())
}

#[test]
fn checkpoint_queued_before_removal_cannot_recreate_deleted_entries() -> Result<()> {
    let t = tempfile::tempdir()?;
    let mut store = Store::open(&t.path().join("db"))?;
    let mut lib = store.import(vec![draft("removed", 1), draft("kept", 1)])?;
    lib.session.current = Some(Target::Book(lib.parts[0].id));
    lib.update_progress(5_000, false);
    store.save(&lib.session, &lib.progress)?;
    store.remove_source(lib.media[0].source_id)?;
    store.save(&lib.session, &lib.progress)?;
    let restored = store.load()?;
    assert!(restored.progress.is_empty());
    assert!(restored.session.current.is_none());
    Ok(())
}

#[test]
fn independent_progress_and_foreign_keys() -> Result<()> {
    let t = tempfile::tempdir()?;
    let mut s = Store::open(&t.path().join("db"))?;
    let mut l = s.import(vec![draft("one", 1), draft("two", 1)])?;
    l.session.current = Some(Target::Book(l.parts[0].id));
    l.update_progress(10_000, false);
    l.session.current = Some(Target::Book(l.parts[1].id));
    l.update_progress(25_000, false);
    s.save(&l.session, &l.progress)?;
    let loaded = s.load()?;
    assert_eq!(loaded.progress.len(), 2);
    assert_eq!(loaded.progress[0].position, 10_000);
    assert_eq!(loaded.progress[1].position, 25_000);
    l.progress[0].part_id = l.parts[1].id;
    assert!(s.save(&l.session, &l.progress).is_err());
    assert_eq!(s.load()?.progress[0].position, 10_000);
    Ok(())
}
