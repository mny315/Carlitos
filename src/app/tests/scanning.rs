use super::*;
use crate::import::ScanResult;

#[test]
fn queued_preview_commands_cannot_modify_or_import_a_new_scans_preview() -> Result<()> {
    for action in ["edit", "file", "import"] {
        let temp = tempfile::tempdir()?;
        let (mut app, commands) = controller(temp.path())?;
        app.drafts = vec![Draft {
            root_uri: app.library.sources[0].uri.clone(),
            title: "Old preview".into(),
            author: String::new(),
            files: app.library.media.clone(),
            include: true,
        }];
        let queued = match action {
            "edit" => Command::DraftEdit(
                app.scan_generation,
                0,
                "Old edited title".into(),
                "Old author".into(),
                false,
            ),
            "file" => Command::DraftFile(
                app.scan_generation,
                0,
                app.library.media[0].uri.clone(),
                "remove".into(),
            ),
            "import" => Command::Import(app.scan_generation, "old owner".into()),
            _ => unreachable!(),
        };
        // A rescan can finish before the controller receives an event from
        // the old preview, including when rescanning the very same directory.
        app.handle(Command::Scan(
            temp.path().join("Книга с пробелами").into(),
            true,
        ))?;
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.drafts.is_empty() {
            app.handle(commands.recv_timeout(deadline.saturating_duration_since(Instant::now()))?)?;
        }
        let expected = app.drafts[0].clone();
        app.handle(queued)?;
        assert_eq!(
            app.drafts.len(),
            1,
            "stale {action} must keep the new preview"
        );
        let current = &app.drafts[0];
        assert_eq!(
            current.title, expected.title,
            "stale {action} changed title"
        );
        assert_eq!(current.author, expected.author);
        assert_eq!(current.include, expected.include);
        assert_eq!(
            current.files.iter().map(|m| &m.uri).collect::<Vec<_>>(),
            expected.files.iter().map(|m| &m.uri).collect::<Vec<_>>()
        );
        // The same action from the current preview must remain usable.
        match action {
            "edit" => {
                app.handle(Command::DraftEdit(
                    app.scan_generation,
                    0,
                    "Current title".into(),
                    "Current author".into(),
                    false,
                ))?;
                assert_eq!(app.drafts[0].title, "Current title");
                assert!(!app.drafts[0].include);
            }
            "file" => {
                app.handle(Command::DraftFile(
                    app.scan_generation,
                    0,
                    expected.files[0].uri.clone(),
                    "remove".into(),
                ))?;
                assert_eq!(app.drafts[0].files.len(), expected.files.len() - 1);
            }
            "import" => {
                let generation = app.scan_generation;
                let (events, received) = async_channel::unbounded();
                app.events = events;
                app.handle(Command::Import(generation, "current owner".into()))?;
                assert!(app.drafts.is_empty());
                assert!(std::iter::from_fn(|| received.try_recv().ok()).any(
                    |event| matches!(event, Event::Imported(saved, owner) if saved == generation && owner == "current owner")
                ));
            }
            _ => unreachable!(),
        }
    }
    Ok(())
}

#[test]
fn a_source_refresh_keeps_controls_busy_until_its_completion_is_published() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let source = app.library.sources[0].id;
    app.scan_source = Some(source);
    let (events, received) = async_channel::unbounded();
    app.events = events;
    app.handle(Command::ScanDone(
        app.scan_generation,
        Ok(ScanResult {
            drafts: vec![Draft {
                root_uri: app.library.sources[0].uri.clone(),
                title: app.library.books[0].title.clone(),
                author: app.library.books[0].author.clone(),
                files: app.library.media.clone(),
                include: true,
            }],
            folders: vec![],
            issues: vec![],
        }),
    ))?;
    let events: Vec<_> = std::iter::from_fn(|| received.try_recv().ok()).collect();
    let completed = events
        .iter()
        .position(|event| matches!(event, Event::SourceUpdated(id, _) if *id == source))
        .expect("source completion");
    let idle = events
        .iter()
        .position(|event| matches!(event, Event::SourceUpdating(false)))
        .expect("source controls released");
    assert!(
        completed < idle,
        "releasing source controls first lets an old completion close a newly opened editor"
    );
    Ok(())
}

#[test]
fn source_refresh_failures_release_controls_without_publishing_success() -> Result<()> {
    for failure in ["scan", "checkpoint", "import"] {
        let temp = tempfile::tempdir()?;
        let (mut app, _commands) = controller(temp.path())?;
        let source = app.library.sources[0].id;
        app.scan_source = Some(source);
        let (events, received) = async_channel::unbounded();
        app.events = events;
        let result = if failure == "scan" {
            Err(anyhow::anyhow!("test scan failure"))
        } else {
            let trigger = if failure == "checkpoint" {
                "CREATE TRIGGER fail_checkpoint BEFORE INSERT ON session_state BEGIN SELECT RAISE(ABORT, 'test checkpoint failure'); END;"
            } else {
                "CREATE TRIGGER fail_import BEFORE UPDATE ON media_files BEGIN SELECT RAISE(ABORT, 'test import failure'); END;"
            };
            rusqlite::Connection::open(temp.path().join("db"))?.execute_batch(trigger)?;
            Ok(ScanResult {
                drafts: vec![Draft {
                    root_uri: app.library.sources[0].uri.clone(),
                    title: app.library.books[0].title.clone(),
                    author: app.library.books[0].author.clone(),
                    files: app.library.media.clone(),
                    include: true,
                }],
                folders: vec![],
                issues: vec![],
            })
        };
        assert!(
            app.handle(Command::ScanDone(app.scan_generation, result))
                .is_err()
        );
        assert!(app.scan_source.is_none());
        let events: Vec<_> = std::iter::from_fn(|| received.try_recv().ok()).collect();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::SourceUpdating(false))),
            "{failure}"
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Scanning(false, _))),
            "{failure}"
        );
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, Event::SourceUpdated(..))),
            "{failure}"
        );
    }
    Ok(())
}

#[test]
fn relocation_keeps_source_controls_busy_through_its_followup_refresh() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let source = app.library.sources[0].id;
    app.maintenance = true;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    app.handle(Command::Relocated(source, Ok(())))?;
    let followup = app.scan_source;
    let events: Vec<_> = std::iter::from_fn(|| received.try_recv().ok()).collect();
    app.scan_control.cancel();
    for worker in app.scans.drain(..) {
        worker.join().expect("source refresh worker");
    }
    assert_eq!(followup, Some(source));
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::SourceUpdating(true)))
    );
    assert!(
        !events
            .iter()
            .any(|event| matches!(event, Event::SourceUpdating(false))),
        "the editor must not reopen between relocation and its refresh"
    );
    Ok(())
}

#[test]
fn background_completions_clear_busy_flags_while_waiting_to_quit() -> Result<()> {
    for quitting in [true, false] {
        let temp = tempfile::tempdir()?;
        let (mut app, _commands) = controller(temp.path())?;
        let source = app.library.sources[0].id;
        app.quitting = quitting;
        app.scan_control.cancel();
        app.maintenance_control.cancel();
        app.scan_source = Some(source);
        app.handle(Command::ScanDone(
            app.scan_generation,
            Err(anyhow::anyhow!("cancelled")),
        ))?;
        assert!(
            app.scan_source.is_none(),
            "a cancelled refresh must not stay busy"
        );
        assert_eq!(app.quitting, quitting);
        app.maintenance = true;
        app.handle(Command::Relocated(
            source,
            Err(anyhow::anyhow!("cancelled")),
        ))?;
        assert!(
            !app.maintenance,
            "a cancelled relocation must not stay busy"
        );
        assert_eq!(app.quitting, quitting);
    }
    Ok(())
}

#[test]
fn relocation_committed_during_shutdown_is_visible_if_exit_is_cancelled() -> Result<()> {
    for save_fails in [false, true] {
        let temp = tempfile::tempdir()?;
        // Windows TEMP can contain a short (8.3) directory alias. Import and
        // relocation return canonical paths, so use the same form in fixtures.
        let root = dunce::canonicalize(temp.path())?;
        let (mut app, _commands) = controller(&root)?;
        let source = app.library.sources[0].id;
        let old = local_path(&app.library.sources[0].uri).unwrap();
        let moved = root.join("moved");
        std::fs::rename(old, &moved)?;
        app.store()?.relocate(source, &moved)?;
        app.library.session.current = Some(Target::Book(app.library.parts[0].id));
        app.library.update_progress(4_000, false);
        if save_fails {
            rusqlite::Connection::open(root.join("db"))?.execute_batch(
                "CREATE TRIGGER reject_checkpoint BEFORE INSERT ON session_state
                 BEGIN SELECT RAISE(FAIL, 'checkpoint failed'); END;",
            )?;
        }
        app.quitting = true;
        app.maintenance = true;
        assert_eq!(
            app.handle(Command::Relocated(source, Ok(()))).is_err(),
            save_fails
        );
        assert!(!app.maintenance);
        assert_eq!(local_path(&app.library.sources[0].uri), Some(moved));
        assert_eq!(app.library.session.position, 4_000);
        assert_eq!(app.library.progress[0].position, 4_000);
        assert!(
            app.scan_source.is_none(),
            "shutdown must not start a new refresh"
        );
    }
    Ok(())
}

#[test]
fn queued_preview_moves_keep_targeting_the_selected_file() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let files = app.library.media.clone();
    let selected = files[2].uri.clone();
    app.drafts = vec![Draft {
        root_uri: app.library.sources[0].uri.clone(),
        title: "Preview".into(),
        author: String::new(),
        files,
        include: true,
    }];
    for _ in 0..2 {
        app.handle(Command::DraftFile(
            app.scan_generation,
            0,
            selected.clone(),
            "up".into(),
        ))?;
    }
    assert_eq!(app.drafts[0].files[0].uri, selected);
    for _ in 0..2 {
        app.handle(Command::DraftFile(
            app.scan_generation,
            0,
            selected.clone(),
            "remove".into(),
        ))?;
    }
    assert_eq!(app.drafts[0].files.len(), 2);
    assert!(app.drafts[0].files.iter().all(|file| file.uri != selected));
    Ok(())
}

#[test]
fn source_refresh_and_relocation_keep_combined_books_and_listening_state() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = dunce::canonicalize(temp.path())?;
    let initial = fixture(&root)?;
    let collection = root.join("Collection");
    let combined = collection.join("Combined");
    std::fs::create_dir(&collection)?;
    std::fs::rename(root.join("Книга с пробелами"), &combined)?;
    let mut store = Store::open(&root.join("db"))?;
    store.relocate(initial.sources[0].id, &combined)?;
    for folder in ["Volume A", "Volume B"] {
        std::fs::create_dir(combined.join(folder))?;
        std::fs::copy(combined.join("1.wav"), combined.join(folder).join("1.wav"))?;
    }
    let scan = import::scan(
        vec![combined.clone()],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    let library = store.import(scan.drafts)?;
    let book = library.books[0].id;
    let order: Vec<_> = library
        .book_parts(book)
        .iter()
        .rev()
        .map(|p| p.id)
        .collect();
    let mut library = store.edit_book(
        book,
        "Custom title".into(),
        "Custom author".into(),
        order.clone(),
        Some("custom.png".into()),
    )?;
    library.session.current = Some(Target::Book(order[0]));
    library.update_progress(5000, false);
    store.save(&library.session, &library.progress)?;
    let scan = import::scan(
        vec![collection.clone()],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    let library = store.import(scan.drafts)?;
    let source = library
        .sources
        .iter()
        .find(|s| local_path(&s.uri).as_ref() == Some(&collection))
        .unwrap()
        .id;
    assert_eq!(library.source_book_counts()[&source], 1);
    std::fs::copy(combined.join("1.wav"), combined.join("4.wav"))?;
    let new_book = collection.join("New book");
    std::fs::create_dir(&new_book)?;
    std::fs::copy(combined.join("1.wav"), new_book.join("1.wav"))?;
    let mut app = start(&root);
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx.send(Command::Rescan(source))?;
    wait(&app, |e| {
        assert!(!matches!(e, Event::Imported(..)));
        if let Event::Drafts(_, drafts, _) = e {
            assert!(drafts.is_empty());
        }
        matches!(e, Event::SourceUpdated(id, _) if *id == source)
    })?;
    let refreshed = store.load()?;
    assert_eq!(refreshed.books.len(), 2);
    assert_eq!(refreshed.source_book_counts()[&source], 2);
    let parts: Vec<_> = refreshed.book_parts(book).iter().map(|p| p.id).collect();
    assert_eq!(&parts[..order.len()], order.as_slice());
    assert_eq!(parts.len(), order.len() + 1);
    let edited = refreshed.books.iter().find(|b| b.id == book).unwrap();
    assert_eq!(
        (&*edited.title, &*edited.author, edited.cover.as_deref()),
        ("Custom title", "Custom author", Some("custom.png"))
    );
    assert_eq!(refreshed.progress[0].position, 5000);
    let moved = root.join("Moved collection");
    // Relinking accepts an existing copy of the collection. On Windows the
    // active decoder holds a file open, preventing a live directory rename.
    for media in &refreshed.media {
        let old = local_path(&media.uri).unwrap();
        let new = moved.join(old.strip_prefix(&collection)?);
        std::fs::create_dir_all(new.parent().unwrap())?;
        std::fs::copy(old, new)?;
    }
    app.tx
        .send(Command::Relocate(source, moved.clone().into()))?;
    wait(
        &app,
        |e| matches!(e, Event::SourceUpdated(id, _) if *id == source),
    )?;
    let relocated = store.load()?;
    assert_eq!(relocated.books.len(), 2);
    assert_eq!(relocated.source_book_counts()[&source], 2);
    assert!(
        relocated
            .sources
            .iter()
            .all(|s| local_path(&s.uri).unwrap().starts_with(&moved))
    );
    assert!(
        relocated
            .media
            .iter()
            .all(|m| local_path(&m.uri).unwrap().is_file())
    );
    assert_eq!(
        relocated
            .book_parts(book)
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        parts
    );
    assert_eq!(relocated.progress[0].position, 5000);
    quit(&mut app)?;
    std::fs::remove_dir_all(&collection)?;
    let mut app = start(&root);
    ready(&app, 5000, false)?;
    quit(&mut app)?;
    Ok(())
}

#[test]
fn import_cancellation_and_stale_scan_cannot_commit() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let _ = fixture(temp.path())?;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx.send(Command::Scan(
        temp.path().join("Книга с пробелами").into(),
        false,
    ))?;
    app.tx.send(Command::CancelScan)?;
    wait(&app, |e| matches!(e, Event::Scanning(false, _)))?;
    app.tx.send(Command::ScanDone(
        1,
        Ok(ScanResult {
            drafts: vec![],
            issues: vec!["stale".into()],
            folders: vec![],
        }),
    ))?;
    quit(&mut app)?;
    assert_eq!(Store::open(&temp.path().join("db"))?.load()?.books.len(), 1);
    Ok(())
}

#[test]
fn relocation_cancellation_keeps_source_and_progress() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let original = fixture(temp.path())?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let control = ScanControl::default();
    control.cancel();
    assert!(
        store
            .relocate_cancellable(original.sources[0].id, temp.path(), &control)
            .is_err()
    );
    let after = store.load()?;
    assert_eq!(after.sources[0].uri, original.sources[0].uri);
    assert_eq!(after.parts.len(), original.parts.len());
    Ok(())
}

#[test]
fn a_same_second_rescan_invalidates_the_previous_content_fingerprint() -> Result<()> {
    use crate::sync::Fingerprint;
    use std::io::{Seek, SeekFrom, Write};
    use std::sync::atomic::AtomicBool;
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let media = &library.media[0];
    let original = Fingerprint::read(media, &AtomicBool::new(false))?;
    let mut store = Store::open(&temp.path().join("db"))?;
    store.save_fingerprints(&library, std::slice::from_ref(&original))?;
    let path = local_path(&media.uri).unwrap();
    let modified = std::fs::metadata(&path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?;
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)?;
    file.seek(SeekFrom::Start(44))?;
    file.write_all(&1234i16.to_le_bytes())?;
    let changed = std::time::UNIX_EPOCH
        + Duration::new(
            modified.as_secs(),
            (modified.subsec_nanos() + 10_000_000) % 1_000_000_000,
        );
    file.set_times(std::fs::FileTimes::new().set_modified(changed))?;
    drop(file);
    assert_eq!(
        Some(crate::platform::modified(&std::fs::metadata(&path)?)),
        media.modified
    );
    let scan = import::scan(
        vec![path.parent().unwrap().to_path_buf()],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    let refreshed = store.import(scan.drafts)?;
    let current = refreshed.media(media.id).unwrap();
    assert!(
        !original.matches(current),
        "a same-size edit within one second must invalidate the cached hash"
    );
    let fingerprint = Fingerprint::read(current, &AtomicBool::new(false))?;
    assert_ne!(fingerprint.hash, original.hash);
    let destination = temp.path().join("relocated-copy");
    std::fs::create_dir(&destination)?;
    for media in &refreshed.media {
        let source = local_path(&media.uri).unwrap();
        let target = destination.join(source.file_name().unwrap());
        std::fs::copy(&source, &target)?;
        let modified = std::fs::metadata(&source)?
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)?;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&target)?
            .set_times(std::fs::FileTimes::new().set_modified(
                std::time::UNIX_EPOCH
                    + Duration::new(
                        modified.as_secs(),
                        (modified.subsec_nanos() + 20_000_000) % 1_000_000_000,
                    ),
            ))?;
    }
    let relocated = store.relocate(refreshed.books[0].source_id, &destination)?;
    let moved = relocated.media(media.id).unwrap();
    assert_eq!(
        Fingerprint::read(moved, &AtomicBool::new(false))?.hash,
        fingerprint.hash
    );
    Ok(())
}
