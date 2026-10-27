use super::*;

#[test]
fn repeated_background_write_failures_report_once_and_rearm_after_recovery() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let mut app = start(temp.path());
    app.tx.send(Command::Part(library.parts[0].id, 0))?;
    app.tx.send(Command::Playing(false))?;
    let snapshot = ready(&app, 0, false)?;
    let conn = rusqlite::Connection::open(temp.path().join("db"))?;
    let reject = "CREATE TRIGGER reject_checkpoint BEFORE INSERT ON session_state
        BEGIN SELECT RAISE(FAIL, 'checkpoint denied'); END;";
    conn.execute_batch(reject)?;
    let first = checkpoint_burst(&app, &snapshot, 1_000)?;
    conn.execute_batch("DROP TRIGGER reject_checkpoint")?;
    let recovered = checkpoint_burst(&app, &snapshot, 2_000)?;
    conn.execute_batch(reject)?;
    let repeated = checkpoint_burst(&app, &snapshot, 3_000)?;
    // Quit obtains its checkpoint asynchronously too, but must explain a
    // refused exit even when this exact background error was already shown.
    app.tx.send(Command::Quit)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut shown, mut refused) = (false, false);
    while !shown || !refused {
        match app.events.try_recv() {
            Ok(Event::Show) => shown = true,
            Ok(Event::Notice(message)) => {
                assert!(
                    message.contains("checkpoint denied"),
                    "unexpected error: {message}"
                );
                refused = true;
            }
            Ok(Event::Quit) => bail!("failed checkpoint must cancel shutdown"),
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {}
            Err(error) => return Err(error.into()),
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "failed Quit did not report its error"
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    conn.execute_batch("DROP TRIGGER reject_checkpoint")?;
    app.tx.send(Command::Quit)?;
    wait(&app, |event| matches!(event, Event::Quit))?;
    app.join();
    assert_eq!(
        first,
        (1, 1),
        "one failure must not repeatedly raise the window"
    );
    assert_eq!(recovered, (0, 0));
    assert_eq!(
        repeated,
        (1, 1),
        "a new failure after recovery must be visible"
    );
    Ok(())
}

fn checkpoint_burst(app: &Handle, snapshot: &Playback, position: u64) -> Result<(usize, usize)> {
    for offset in 0..4 {
        app.tx.send(Command::Playback(Playback {
            position: position + offset,
            ..snapshot.clone()
        }))?;
    }
    // This command runs on the same controller queue, after every snapshot.
    const MARKER: &str = "audit checkpoint burst processed";
    app.tx.send(Command::Error(MARKER.into()))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    let (mut shown, mut notices) = (0, 0);
    loop {
        match app.events.try_recv() {
            Ok(Event::Show) => shown += 1,
            Ok(Event::Notice(message)) if message == MARKER => return Ok((shown, notices)),
            Ok(Event::Notice(message)) => {
                assert!(
                    message.contains("checkpoint denied"),
                    "unexpected error: {message}"
                );
                notices += 1;
            }
            Ok(_) | Err(async_channel::TryRecvError::Empty) => {}
            Err(error) => return Err(error.into()),
        }
        anyhow::ensure!(Instant::now() < deadline, "checkpoint burst timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn a_checkpoint_write_failure_does_not_hide_the_confirmed_pause_from_the_view() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _) = controller(temp.path())?;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    app.library.session.current = Some(Target::Book(app.library.parts[0].id));
    app.playback.playing = true;
    app.pending_save = true;
    let conn = rusqlite::Connection::open(temp.path().join("db"))?;
    conn.execute_batch(
        "CREATE TRIGGER reject_checkpoint BEFORE INSERT ON session_state
         BEGIN SELECT RAISE(FAIL, 'checkpoint failed'); END;",
    )?;
    let snapshot = Playback {
        phase: crate::player::Phase::Ready,
        position: 4_000,
        playing: false,
        ..Default::default()
    };
    assert!(app.handle(Command::Playback(snapshot.clone())).is_err());
    let mut paused = false;
    while let Ok(event) = received.try_recv() {
        if let Event::Playback(p) = event {
            paused |= p.position == 4_000 && !p.playing;
        }
    }
    assert!(
        paused,
        "persistence failure must not freeze the pause button and clock"
    );
    assert_eq!(app.library.progress[0].position, 4_000);
    conn.execute_batch("DROP TRIGGER reject_checkpoint;")?;
    app.handle(Command::Playback(snapshot))?;
    assert_eq!(app.store()?.load()?.progress[0].position, 4_000);
    Ok(())
}

#[test]
fn stopping_during_a_checkpoint_failure_still_stops_audio_and_keeps_progress_for_retry()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, commands) = controller(temp.path())?;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    app.load(app.library.parts[0].id, 3_000, true)?;
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.playback.phase != crate::player::Phase::Ready {
        app.handle(commands.recv_timeout(deadline.saturating_duration_since(Instant::now()))?)?;
    }
    let stale = Playback {
        position: 4_000,
        ..app.playback.clone()
    };
    app.handle(Command::Playback(stale.clone()))?;
    let conn = rusqlite::Connection::open(temp.path().join("db"))?;
    conn.execute_batch(
        "CREATE TRIGGER reject_checkpoint BEFORE INSERT ON session_state
         BEGIN SELECT RAISE(FAIL, 'checkpoint failed'); END;",
    )?;
    assert!(app.handle(Command::Stop).is_err());
    assert!(
        !app.playback.playing,
        "a write failure must not prevent Stop"
    );
    assert_eq!(app.library.session.current, None);
    assert_eq!(app.library.progress[0].position, 4_000);
    let mut cleared = false;
    while let Ok(event) = received.try_recv() {
        if let Event::Library(library) = event {
            cleared |= library.session.current.is_none();
        }
    }
    assert!(
        cleared,
        "the UI must receive the stopped selection despite the save error"
    );
    // A decoder snapshot queued before Stop must not revive its selection.
    app.handle(Command::Playback(stale))?;
    assert_eq!(app.playback.phase, crate::player::Phase::Empty);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let command = commands.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
        if matches!(&command, Command::Playback(p) if p.token == app.token && p.phase == crate::player::Phase::Empty)
        {
            break;
        }
    }
    conn.execute_batch("DROP TRIGGER reject_checkpoint;")?;
    app.save()?;
    let library = app.store()?.load()?;
    assert_eq!(library.session.current, None);
    assert_eq!(library.progress[0].position, 4_000);
    Ok(())
}

#[test]
fn restoring_a_paused_position_does_not_create_a_new_listening_revision() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    let part = app.library.parts[0].id;
    app.library.session.current = Some(Target::Book(part));
    app.library.update_progress(3_123, false);
    let updated = app.library.progress[0].updated;
    app.save()?;
    app.load(part, 3_123, false)?;
    // Different decoders may round their initial seek to a frame boundary.
    for position in [3_120, 3_123, 3_120] {
        app.handle(Command::Playback(Playback {
            token: app.token,
            position,
            phase: crate::player::Phase::Ready,
            seekable: true,
            ..Default::default()
        }))?;
        let mut restored = false;
        while let Ok(event) = received.try_recv() {
            if let Event::Playback(p) = event {
                restored |= p.preserve_progress;
            }
        }
        assert!(
            restored,
            "the view must preserve this restored checkpoint too"
        );
    }
    app.save()?;
    let saved = app.store()?.load()?;
    assert_eq!(saved.progress[0].updated, updated);
    assert_eq!(saved.progress[0].position, 3_123);
    app.seek_to(1_000);
    app.handle(Command::Playback(Playback {
        token: app.token,
        position: 1_000,
        seek_done: true,
        phase: crate::player::Phase::Ready,
        ..Default::default()
    }))?;
    while let Ok(event) = received.try_recv() {
        if let Event::Playback(p) = event {
            assert!(!p.preserve_progress, "an explicit seek changes progress");
        }
    }
    let saved = app.store()?.load()?;
    assert_eq!(saved.progress[0].position, 1_000);
    assert!(saved.progress[0].updated > updated);
    Ok(())
}

#[test]
fn unavailable_database_and_audio_together_do_not_prevent_quit() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let path = temp.path().join("damaged-db");
    let original = b"not a SQLite database";
    std::fs::write(&path, original)?;
    app.store = Some(Store::open(&path));
    assert!(app.store.as_ref().unwrap().is_err());
    app.library = Library::default();
    app.quitting = true;
    assert!(app.handle(Command::AudioFailed(0, "Service unavailable".into()))?);
    assert_eq!(std::fs::read(path)?, original);
    Ok(())
}

#[test]
fn paused_part_selection_is_saved_on_the_first_confirmed_snapshot() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let part = app.library.parts[1].id;
    app.load(part, 3_000, false)?;
    app.handle(Command::Playback(Playback {
        token: app.token,
        position: 3_000,
        phase: crate::player::Phase::Ready,
        ..Default::default()
    }))?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.current, Some(Target::Book(part)));
    assert_eq!(saved.session.position, 3_000);
    assert_eq!(saved.progress[0].position, 3_000);
    Ok(())
}

#[test]
fn audio_error_and_service_loss_flush_only_confirmed_progress() -> Result<()> {
    for service_loss in [false, true] {
        let temp = tempfile::tempdir()?;
        let (mut app, _commands) = controller(temp.path())?;
        app.library.session.current = Some(Target::Book(app.library.parts[0].id));
        app.handle(Command::Playback(Playback {
            position: 3_000,
            playing: true,
            phase: crate::player::Phase::Ready,
            ..Default::default()
        }))?;
        assert!(
            Store::open(&temp.path().join("db"))?
                .load()?
                .progress
                .is_empty()
        );
        if service_loss {
            app.handle(Command::AudioStopped)?;
        } else {
            app.handle(Command::Playback(Playback {
                position: 0,
                phase: crate::player::Phase::Error,
                error: Some("Source disappeared".into()),
                ..Default::default()
            }))?;
        }
        let saved = Store::open(&temp.path().join("db"))?.load()?;
        assert_eq!(saved.session.position, 3_000);
        assert_eq!(saved.progress[0].position, 3_000);
        assert!(!saved.progress[0].completed);
    }
    Ok(())
}

#[test]
fn failed_chapter_load_does_not_save_an_unconfirmed_position() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let part = app.library.parts[0].id;
    app.library.session.current = Some(Target::Book(part));
    app.library.update_progress(3_000, false);
    app.load(part, 9_000, true)?;
    app.handle(Command::Playback(Playback {
        token: app.token,
        position: 9_000,
        phase: crate::player::Phase::Error,
        error: Some("Source disappeared".into()),
        ..Default::default()
    }))?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.position, 3_000);
    assert_eq!(saved.progress[0].position, 3_000);
    app.handle(Command::Playing(true))?;
    assert_eq!(app.playback.position, 3_000);
    Ok(())
}

#[test]
fn unsaved_appearance_settings_still_match_the_visible_ui() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    std::fs::create_dir(&app.options.settings_path)?;
    let settings = Settings {
        theme: "light".into(),
        close_to_tray: false,
        library_sort: "author".into(),
        ..app.options.settings.clone()
    };
    assert!(app.handle(Command::Settings(settings.clone())).is_err());
    assert_eq!(app.options.settings, settings);
    Ok(())
}

#[test]
fn a_settings_write_failure_does_not_block_playback_completion() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    app.library.session.current = Some(Target::Book(app.library.parts[2].id));
    // Persisting a regular file over a directory fails on every test host.
    std::fs::create_dir(&app.options.settings_path)?;
    app.handle(Command::Playback(Playback {
        phase: crate::player::Phase::Ready,
        playing: true,
        ended: true,
        position: 12_000,
        duration: Some(12_000),
        rate: 1.5,
        ..Default::default()
    }))?;
    assert!(app.library.progress[0].completed);
    assert!(!app.playback.playing);
    assert!(Store::open(&temp.path().join("db"))?.load()?.progress[0].completed);
    Ok(())
}

#[test]
fn a_book_speed_write_failure_does_not_block_progress_or_shutdown() -> Result<()> {
    for quitting in [false, true] {
        let temp = tempfile::tempdir()?;
        let (mut app, _) = controller(temp.path())?;
        let book = app.library.books[0].id;
        let part = app.library.parts[0].id;
        app.library.session.current = Some(Target::Book(part));
        app.pending_rate = Some((book, 1.5));
        app.pending_save = true;
        app.quitting = quitting;
        let conn = rusqlite::Connection::open(temp.path().join("db"))?;
        conn.execute_batch("CREATE TRIGGER fail_speed BEFORE INSERT ON book_rates BEGIN SELECT RAISE(ABORT, 'test speed write failure'); END;")?;
        let snapshot = Playback {
            phase: crate::player::Phase::Ready,
            position: 4_000,
            rate: 1.5,
            barrier: true,
            ..Default::default()
        };
        assert_eq!(app.handle(Command::Playback(snapshot.clone()))?, quitting);
        let saved = app.store()?.load()?;
        assert_eq!(saved.session.position, 4_000);
        assert_eq!(saved.progress[0].position, 4_000);
        assert_eq!(app.pending_rate, Some((book, 1.5)));
        if !quitting {
            conn.execute_batch("DROP TRIGGER fail_speed;")?;
            app.handle(Command::Playback(snapshot))?;
            assert_eq!(app.store()?.book_rate(book)?, Some(1.5));
            assert!(app.pending_rate.is_none());
        }
    }
    Ok(())
}

#[test]
fn damaged_database_is_preserved_and_does_not_prevent_quit() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let original = b"This is not a SQLite database";
    std::fs::write(temp.path().join("db"), original)?;
    let mut app = start(temp.path());
    app.tx.send(Command::Quit)?;
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        if matches!(app.events.try_recv(), Ok(Event::Quit)) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "damaged database blocked shutdown"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    app.join();
    assert_eq!(std::fs::read(temp.path().join("db"))?, original);
    Ok(())
}
