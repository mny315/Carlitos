use super::super::*;

#[test]
fn desktop_seek_for_an_old_part_cannot_seek_the_new_selection() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let old = app.library.parts[0].id;
    let current = app.library.parts[1].id;
    app.library.session.current = Some(Target::Book(current));
    app.playback.phase = crate::player::Phase::Ready;
    app.playback.seekable = true;
    app.playback.duration = Some(12_000);
    let uri = app.library.active_file().unwrap().uri.clone();
    app.handle(Command::SetPosition(old, uri.clone(), 5_000))?;
    assert_eq!(app.token, 0);
    assert_eq!(app.playback.position, 0);
    app.handle(Command::SetPosition(
        current,
        "file:///removed.wav".into(),
        5_000,
    ))?;
    assert_eq!(app.token, 0);
    assert_eq!(app.playback.position, 0);
    app.handle(Command::SetPosition(current, uri, 5_000))?;
    assert_eq!(app.token, 1);
    assert_eq!(app.playback.position, 5_000);
    Ok(())
}

#[test]
fn a_settled_pause_position_replaces_the_optimistic_checkpoint() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    app.library.session.current = Some(Target::Book(app.library.parts[0].id));
    app.playback.playing = true;
    app.handle(Command::Playing(false))?;
    let mut snapshot = Playback {
        token: app.token,
        phase: crate::player::Phase::Ready,
        position: 1000,
        ..Default::default()
    };
    app.handle(Command::Playback(snapshot.clone()))?;
    assert_eq!(app.store()?.load()?.session.position, 1000);
    // Media3's audio thread can acknowledge its final clock after the main
    // thread has already published playWhenReady=false.
    snapshot.position = 1060;
    app.handle(Command::Playback(snapshot))?;
    assert_eq!(app.store()?.load()?.session.position, 1060);
    Ok(())
}

#[test]
fn controller_restores_paused_and_resume_uses_live_position() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let book = library.books[0].id;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx.send(Command::Resume(book))?;
    ready(&app, 0, true)?;
    app.tx.send(Command::SeekAbsolute(5_000))?;
    ready(&app, 5_000, true)?;
    app.tx.send(Command::Playing(false))?;
    let paused = ready(&app, 5_000, false)?;
    app.tx.send(Command::Resume(book))?;
    let resumed = ready(&app, paused.position, true)?;
    assert!(resumed.position >= paused.position.saturating_sub(50));
    quit(&mut app)?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert!(saved.session.position >= 5_000);
    assert_eq!(saved.progress[0].part_id, library.parts[0].id);
    let mut restored = start(temp.path());
    ready(&restored, saved.session.position, false)?;
    quit(&mut restored)?;
    Ok(())
}

#[test]
fn playback_retries_a_restored_file_without_losing_the_checkpoint() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut library = fixture(temp.path())?;
    let book = library.books[0].id;
    library.session.current = Some(Target::Book(library.parts[0].id));
    library.update_progress(5_000, false);
    Store::open(&temp.path().join("db"))?.save(&library.session, &library.progress)?;
    let path = local_path(&library.media[0].uri).unwrap();
    let moved = path.with_extension("moved");
    std::fs::rename(&path, &moved)?;
    let mut app = start(temp.path());
    let deadline = Instant::now() + Duration::from_secs(12);
    loop {
        if matches!(app.events.try_recv(), Ok(Event::Playback(p)) if p.phase == crate::player::Phase::Error)
        {
            break;
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "Missing file did not report an error"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    std::fs::rename(moved, path)?;
    app.tx.send(Command::Resume(book))?;
    // The failed pipeline can still have error notices queued when retry starts.
    let deadline = Instant::now() + Duration::from_secs(12);
    let mut resumed = false;
    while Instant::now() < deadline {
        if matches!(app.events.try_recv(), Ok(Event::Playback(p)) if p.phase == crate::player::Phase::Ready && p.playing && p.position.abs_diff(5_000) < 600)
        {
            resumed = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    quit(&mut app)?;
    assert!(
        resumed,
        "Restored file could not resume from its checkpoint"
    );
    let restored = Store::open(&temp.path().join("db"))?.load()?;
    assert!(restored.progress[0].position >= 5_000);
    Ok(())
}

#[test]
fn rapid_seek_crosses_parts_and_removed_book_stays_removed() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let book = library.books[0].id;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx.send(Command::Resume(book))?;
    ready(&app, 0, true)?;
    app.tx.send(Command::Playing(false))?;
    ready(&app, 0, false)?;
    app.tx.send(Command::SeekAbsolute(10_000))?;
    ready(&app, 10_000, false)?;
    app.tx.send(Command::SeekDelta(5_000))?;
    ready(&app, 3_000, false)?;
    app.tx.send(Command::SeekAbsolute(2_000))?;
    app.tx.send(Command::SeekAbsolute(8_000))?;
    app.tx.send(Command::SeekAbsolute(4_000))?;
    ready(&app, 4_000, false)?;
    app.tx.send(Command::RemoveBook(book))?;
    wait(
        &app,
        |e| matches!(e, Event::Library(l) if l.books.is_empty()),
    )?;
    quit(&mut app)?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert!(saved.books.is_empty() && saved.progress.is_empty() && saved.session.current.is_none());
    Ok(())
}

#[test]
fn immediate_quit_after_seek_saves_the_final_confirmed_target() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx.send(Command::Resume(library.books[0].id))?;
    ready(&app, 0, true)?;
    app.tx.send(Command::Playing(false))?;
    let paused = ready(&app, 0, false)?;
    app.tx.send(Command::SeekDelta(15_000))?;
    app.tx.send(Command::SeekDelta(1_000))?;
    quit(&mut app)?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(
        saved.session.current,
        Some(Target::Book(library.parts[1].id))
    );
    assert!(
        saved.session.position.abs_diff(paused.position + 4_000) < 150,
        "{}",
        saved.session.position
    );
    Ok(())
}
