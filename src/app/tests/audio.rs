use super::*;

#[test]
fn transport_failures_are_retryable_and_ignore_superseded_commands() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let part = app.library.parts[0].id;
    app.library.session.current = Some(Target::Book(part));
    app.library.update_progress(3_000, false);
    app.load(part, 9_000, true)?;
    let token = app.token;
    app.handle(Command::AudioFailed(token - 1, "Old startup failed".into()))?;
    assert_eq!(app.playback.phase, crate::player::Phase::Loading);
    app.handle(Command::AudioFailed(
        token,
        "Service refused to start".into(),
    ))?;
    assert_eq!(app.playback.phase, crate::player::Phase::Error);
    assert!(!app.playback.playing && !app.playback.seekable);
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.position, 3_000);
    assert_eq!(saved.progress[0].position, 3_000);
    app.handle(Command::Playing(true))?;
    assert_eq!(app.playback.phase, crate::player::Phase::Loading);
    assert_eq!(app.playback.position, 3_000);
    app.handle(Command::AudioFailed(token, "Late startup failure".into()))?;
    assert_eq!(app.playback.phase, crate::player::Phase::Loading);
    // If the transport fails during Quit, there will be no worker snapshot.
    app.quitting = true;
    assert!(app.handle(Command::AudioFailed(
        app.token,
        "Transport unavailable".into()
    ))?);
    Ok(())
}

#[test]
fn choosing_a_book_after_audio_shutdown_does_not_enter_loading() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let book = app.library.books[0].id;
    app.audio.stop();
    app.handle(Command::AudioStopped)?;
    assert!(app.handle(Command::Resume(book)).is_err());
    assert_eq!(app.playback.phase, crate::player::Phase::Error);
    assert!(app.library.session.current.is_none());
    assert_eq!(app.token, 0);
    Ok(())
}

#[test]
fn audio_retains_eos_when_a_new_command_overtakes_its_snapshot() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let (tx, rx) = mpsc::channel();
    let audio = Audio::start(tx, true);
    audio.send(AudioCommand::Load {
        token: 1,
        uri: library.media[0].uri.clone(),
        position: 11_900,
        playing: true,
    });
    loop {
        if matches!(rx.recv_timeout(Duration::from_secs(5))?, Command::Playback(p) if p.ended) {
            break;
        }
    }
    audio.send(AudioCommand::Playing(2, false));
    loop {
        if let Command::Playback(p) = rx.recv_timeout(Duration::from_secs(5))?
            && p.token == 2
        {
            assert!(
                p.ended,
                "EOS must survive rejection of the older token's snapshot"
            );
            break;
        }
    }
    Ok(())
}

#[test]
fn audio_error_survives_a_new_command_and_clears_when_retrying() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let missing_uri = file_uri(&temp.path().join("missing.wav"))?;
    let (tx, rx) = mpsc::channel();
    let audio = Audio::start(tx, true);
    let snapshot = |token, phase| -> Result<Playback> {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if let Command::Playback(p) = rx.recv_timeout(remaining)?
                && p.token == token
                && p.phase == phase
            {
                return Ok(p);
            }
        }
    };
    audio.send(AudioCommand::Load {
        token: 1,
        uri: missing_uri,
        position: 0,
        playing: true,
    });
    assert!(snapshot(1, crate::player::Phase::Error)?.error.is_some());
    audio.send(AudioCommand::Playing(2, true));
    let failed = snapshot(2, crate::player::Phase::Error)?;
    assert!(
        failed.error.is_some(),
        "a later command must retain the reason playback failed"
    );
    assert!(!failed.playing, "a failed pipeline cannot resume playback");
    audio.send(AudioCommand::Load {
        token: 3,
        uri: library.media[0].uri.clone(),
        position: 0,
        playing: false,
    });
    assert!(snapshot(3, crate::player::Phase::Ready)?.error.is_none());
    Ok(())
}

#[test]
fn a_missing_file_does_not_keep_restarting_the_failed_pipeline() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (tx, rx) = mpsc::channel();
    let audio = Audio::start(tx, true);
    audio.send(AudioCommand::Load {
        token: 1,
        uri: file_uri(&temp.path().join("missing.wav"))?,
        position: 0,
        playing: true,
    });
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut errors = 0;
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Command::Playback(p)) if p.error.is_some() => errors += 1,
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) if errors > 0 => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
        anyhow::ensure!(
            errors < 32,
            "a missing file causes an endless error/restart loop"
        );
        anyhow::ensure!(
            Instant::now() < deadline,
            "audio worker did not settle after a missing file"
        );
    }
    Ok(())
}

#[test]
fn retained_audio_errors_are_reported_once_after_a_stale_snapshot() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    app.token = 2;
    let mut snapshot = Playback {
        token: 1,
        phase: crate::player::Phase::Error,
        error: Some("Missing audio file".into()),
        ..Default::default()
    };
    app.handle(Command::Playback(snapshot.clone()))?;
    assert!(received.is_empty(), "stale snapshots are ignored");
    snapshot.token = 2;
    app.handle(Command::Playback(snapshot.clone()))?;
    app.token = 3;
    snapshot.token = 3;
    app.handle(Command::Playback(snapshot))?;
    let mut notices = vec![];
    while let Ok(event) = received.try_recv() {
        if let Event::Notice(message) = event {
            notices.push(message);
        }
    }
    assert_eq!(notices, ["Missing audio file"]);
    Ok(())
}
