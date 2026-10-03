use super::super::*;

#[test]
fn system_play_restarts_a_completed_book_at_its_first_part() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let first = app.library.parts[0].id;
    app.library.session.current = Some(Target::Book(app.library.parts[2].id));
    app.library.update_progress(12_000, true);
    app.playback.phase = crate::player::Phase::Ready;
    app.handle(Command::Playing(true))?;
    assert_eq!(app.library.session.current, Some(Target::Book(first)));
    assert_eq!(app.library.session.position, 0);
    assert!(app.playback.playing);
    Ok(())
}

#[test]
fn completion_without_a_known_duration_survives_later_snapshots() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let book = app.library.books[0].id;
    let first = app.library.parts[0].id;
    let last = app.library.parts.last().unwrap().id;
    app.library.session.current = Some(Target::Book(last));
    let mut snapshot = Playback {
        phase: crate::player::Phase::Ready,
        playing: true,
        ended: true,
        position: 12_000,
        duration: None,
        ..Default::default()
    };
    app.handle(Command::Playback(snapshot.clone()))?;
    assert!(app.library.progress[0].completed);
    // Pausing at EOS, changing volume and requesting a checkpoint all send
    // further snapshots, including when the decoder cannot report a duration.
    snapshot.token = app.token;
    snapshot.playing = false;
    for _ in 0..3 {
        app.handle(Command::Playback(snapshot.clone()))?;
        assert!(app.library.progress[0].completed);
    }
    app.save()?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert!(saved.progress[0].completed);
    assert_eq!(saved.progress[0].position, 12_000);
    app.handle(Command::Resume(book))?;
    assert_eq!(app.library.session.current, Some(Target::Book(first)));
    app.handle(Command::Playback(Playback {
        token: app.token,
        phase: crate::player::Phase::Ready,
        playing: true,
        ..Default::default()
    }))?;
    assert!(!app.library.progress[0].completed);
    Ok(())
}

#[test]
fn shutdown_at_eos_checkpoints_completion_or_the_next_part() -> Result<()> {
    for final_part in [false, true] {
        let temp = tempfile::tempdir()?;
        let (mut app, _commands) = controller(temp.path())?;
        let part = app.library.parts[if final_part { 2 } else { 0 }].id;
        let next = app.library.parts[1].id;
        app.library.session.current = Some(Target::Book(part));
        app.quitting = true;
        assert!(app.handle(Command::Playback(Playback {
            position: 12_000,
            duration: Some(12_000),
            playing: true,
            phase: crate::player::Phase::Ready,
            ended: true,
            barrier: true,
            ..Default::default()
        }))?);
        let saved = Store::open(&temp.path().join("db"))?.load()?;
        assert_eq!(saved.progress[0].completed, final_part);
        assert_eq!(
            saved.progress[0].part_id,
            if final_part { part } else { next }
        );
        assert_eq!(saved.session.position, if final_part { 12_000 } else { 0 });
    }
    Ok(())
}

#[test]
fn failed_shutdown_at_eos_does_not_skip_the_next_part() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let first = app.library.parts[0].id;
    let next = app.library.parts[1].id;
    app.library.session.current = Some(Target::Book(first));
    app.quitting = true;
    std::fs::create_dir(&app.options.settings_path)?;
    let mut snapshot = Playback {
        phase: crate::player::Phase::Ready,
        playing: true,
        ended: true,
        barrier: true,
        position: 12_000,
        duration: Some(12_000),
        ..Default::default()
    };
    assert!(app.handle(Command::Playback(snapshot.clone())).is_err());
    // The worker returns control to the window after reporting a save error.
    app.quitting = false;
    snapshot.barrier = false;
    app.handle(Command::Playback(snapshot))?;
    assert_eq!(app.library.session.current, Some(Target::Book(next)));
    assert_eq!(app.library.session.position, 0);
    Ok(())
}

#[test]
fn a_failed_checkpoint_does_not_consume_the_end_of_a_part() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    app.library.session.current = Some(Target::Book(app.library.parts[0].id));
    let next = app.library.parts[1].id;
    let snapshot = Playback {
        phase: crate::player::Phase::Ready,
        playing: true,
        ended: true,
        position: 12_000,
        duration: Some(12_000),
        ..Default::default()
    };
    app.store = Some(Err(anyhow::anyhow!("temporary storage failure")));
    assert!(app.handle(Command::Playback(snapshot.clone())).is_err());
    app.store = Some(Ok(Store::open(&temp.path().join("db"))?));
    app.handle(Command::Playback(snapshot))?;
    assert_eq!(app.library.session.current, Some(Target::Book(next)));
    Ok(())
}

#[test]
fn final_eos_completion_survives_pause_and_reopen() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx
        .send(Command::Part(library.parts.last().unwrap().id, 11_700))?;
    wait(
        &app,
        |e| matches!(e, Event::Library(l) if l.progress.iter().any(|p| p.completed)),
    )?;
    std::thread::sleep(Duration::from_millis(300));
    quit(&mut app)?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert!(saved.progress[0].completed);
    let mut restored = start(temp.path());
    ready(&restored, 12_000, false)?;
    restored.tx.send(Command::Resume(library.books[0].id))?;
    wait(
        &restored,
        |e| matches!(e, Event::Library(l) if l.session.current == Some(Target::Book(library.parts[0].id))),
    )?;
    ready(&restored, 0, true)?;
    quit(&mut restored)?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(
        saved.session.current,
        Some(Target::Book(library.parts[0].id))
    );
    assert!(!saved.progress[0].completed);
    Ok(())
}

#[test]
fn eos_queued_after_pause_does_not_start_the_next_part() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx.send(Command::Resume(library.books[0].id))?;
    ready(&app, 0, true)?;
    app.tx.send(Command::Playing(false))?;
    let mut paused = ready(&app, 0, false)?;
    paused.position = 12_000;
    paused.ended = true;
    app.tx.send(Command::Playback(paused))?;
    wait(
        &app,
        |e| matches!(e, Event::Library(l) if l.session.current == Some(Target::Book(library.parts[1].id))),
    )?;
    ready(&app, 0, false)?;
    quit(&mut app)?;
    Ok(())
}
