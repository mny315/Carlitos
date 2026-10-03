use super::*;
use crate::import::ScanResult;

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
        app.handle(Command::DraftFile(0, selected.clone(), "up".into()))?;
    }
    assert_eq!(app.drafts[0].files[0].uri, selected);
    for _ in 0..2 {
        app.handle(Command::DraftFile(0, selected.clone(), "remove".into()))?;
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
        assert!(!matches!(e, Event::Imported));
        if let Event::Drafts(drafts, _) = e {
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
