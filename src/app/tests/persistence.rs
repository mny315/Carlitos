use super::*;

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
