use super::*;

#[test]
fn volume_changes_preserve_the_checkpoint_without_publishing_the_catalog() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    app.library.session.current = Some(Target::Book(app.library.parts[0].id));
    app.library.update_progress(3_456, false);
    let (events, received) = async_channel::unbounded();
    app.events = events;
    for step in 0..=20 {
        let volume = f64::from(step) / 20.;
        app.handle(Command::Volume(volume))?;
        assert!(matches!(received.try_recv()?, Event::Volume(level, false) if level == volume));
    }
    app.handle(Command::Mute)?;
    assert!(matches!(received.try_recv()?, Event::Volume(1., true)));
    app.handle(Command::Volume(0.4))?;
    assert!(matches!(received.try_recv()?, Event::Volume(0.4, true)));
    app.handle(Command::Mute)?;
    assert!(matches!(received.try_recv()?, Event::Volume(0.4, false)));
    for volume in [0.4, f64::NAN, f64::INFINITY] {
        app.handle(Command::Volume(volume))?;
    }
    assert!(
        received.is_empty(),
        "volume updates must not carry a full library snapshot"
    );
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.volume, 0.4);
    assert!(!saved.session.muted);
    assert_eq!(saved.session.position, 3_456);
    assert_eq!(saved.progress[0].position, 3_456);
    Ok(())
}

#[test]
fn failed_volume_checkpoint_still_reports_the_applied_audio_level() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    app.save()?;
    let connection = rusqlite::Connection::open(temp.path().join("db"))?;
    connection.execute_batch(
        "CREATE TRIGGER reject_session BEFORE UPDATE ON session_state BEGIN SELECT RAISE(FAIL,'checkpoint rejected'); END;",
    )?;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    assert!(app.handle(Command::Volume(0.2)).is_err());
    assert_eq!(app.library.session.volume, 0.2);
    assert!(matches!(received.try_recv()?, Event::Volume(0.2, false)));
    assert!(app.handle(Command::Mute).is_err());
    assert!(matches!(received.try_recv()?, Event::Volume(0.2, true)));
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.volume, 0.7);
    assert!(!saved.session.muted);
    connection.execute_batch("DROP TRIGGER reject_session")?;
    app.save()?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.volume, 0.2);
    assert!(saved.session.muted);
    Ok(())
}
