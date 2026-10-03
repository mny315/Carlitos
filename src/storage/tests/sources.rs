use super::*;

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
