use super::super::*;

#[test]
fn each_book_restores_its_speed_across_switches_and_restart() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let first = library.parts[0].id;
    let first_book = library.books[0].id;
    let folder = temp.path().join("Second book");
    std::fs::create_dir(&folder)?;
    std::fs::copy(
        local_path(&library.media[0].uri).unwrap(),
        folder.join("part.wav"),
    )?;
    let scan = import::scan(vec![folder], ImportMode::Book, ScanControl::default())?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let library = store.import(scan.drafts)?;
    let second = library
        .parts
        .iter()
        .find(|p| p.book_id != first_book)
        .unwrap();
    store.save_book_rate(first_book, 1.25)?;
    store.save_book_rate(second.book_id, 2.0)?;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    for (part, rate) in [(first, 1.25), (second.id, 2.0), (first, 1.25)] {
        app.tx.send(Command::Part(part, 0))?;
        wait(
            &app,
            |e| matches!(e, Event::Library(l) if l.session.current == Some(Target::Book(part))),
        )?;
        assert_eq!(ready(&app, 0, true)?.rate, rate);
    }
    app.tx.send(Command::Rate(1.75))?;
    wait(
        &app,
        |e| matches!(e, Event::Playback(p) if p.rate == 1.75 && p.phase == crate::player::Phase::Ready),
    )?;
    quit(&mut app)?;
    assert_eq!(store.book_rate(first_book)?, Some(1.75));
    assert_eq!(store.book_rate(second.book_id)?, Some(2.0));
    let position = store.load()?.session.position;
    let mut restored = start(temp.path());
    assert_eq!(ready(&restored, position, false)?.rate, 1.75);
    quit(&mut restored)?;
    Ok(())
}

#[test]
fn pending_audio_preferences_survive_a_load_before_their_acknowledgement() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let part = app.library.parts[0].id;
    app.handle(Command::Rate(1.75))?;
    app.handle(Command::RateStep(true))?;
    app.handle(Command::SkipSilence(true))?;
    // The controller has not consumed any audio snapshots yet. Opening a part
    // must carry the user's latest requests, not the last persisted settings.
    app.handle(Command::Part(part, 0))?;
    assert_eq!(app.playback.rate, 1.8);
    assert!(app.playback.skip_silence);
    assert_eq!(app.options.settings.playback_rate, 1.0);
    assert!(!app.options.settings.skip_silence);
    Ok(())
}

#[test]
fn startup_acknowledges_saved_audio_preferences_together() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut app = start_with_settings(
        temp.path(),
        Settings {
            playback_rate: 1.75,
            skip_silence: true,
            ..Settings::default()
        },
    );
    let Event::Playback(first) = wait(&app, |event| matches!(event, Event::Playback(_)))? else {
        unreachable!()
    };
    quit(&mut app)?;
    assert_eq!(first.rate, 1.75);
    assert!(
        first.skip_silence,
        "Published a partially applied startup preference"
    );
    Ok(())
}

#[test]
fn speed_survives_seeking_part_changes_and_restart() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx.send(Command::Rate(1.75))?;
    wait(
        &app,
        |e| matches!(e, Event::Settings(s) if s.playback_rate == 1.75),
    )?;
    app.tx.send(Command::Resume(library.books[0].id))?;
    assert_eq!(ready(&app, 0, true)?.rate, 1.75);
    app.tx.send(Command::Playing(false))?;
    // Playback may advance during initial buffering; only the later explicit
    // seek promises an exact source position.
    wait(
        &app,
        |e| matches!(e, Event::Playback(p) if !p.playing && p.phase == crate::player::Phase::Ready),
    )?;
    app.tx.send(Command::SeekAbsolute(4_000))?;
    app.tx.send(Command::Rate(2.0))?;
    wait(
        &app,
        |e| matches!(e, Event::Playback(p) if p.rate == 2.0 && p.phase == crate::player::Phase::Ready && p.position.abs_diff(4_000) < 150 && !p.playing),
    )?;
    for (forward, expected) in [(true, 2.1), (false, 2.0)] {
        app.tx.send(Command::RateStep(forward))?;
        app.tx.send(Command::RateStep(forward))?;
        wait(&app, |e| {
            matches!(e, Event::Playback(p)
            if p.rate == expected && p.phase == crate::player::Phase::Ready
                && p.position.abs_diff(4_000) < 150 && !p.playing)
        })?;
    }
    app.tx.send(Command::Next(true))?;
    assert_eq!(ready(&app, 0, false)?.rate, 2.0);
    quit(&mut app)?;
    let (settings, warning, writable) = Settings::load(&temp.path().join("settings.json"));
    assert!(warning.is_none() && writable);
    assert_eq!(settings.playback_rate, 2.0);
    let mut restored = start_with_settings(temp.path(), settings);
    assert_eq!(ready(&restored, 0, false)?.rate, 2.0);
    quit(&mut restored)?;
    Ok(())
}

#[test]
fn silence_skip_survives_part_changes_and_restart_with_source_position() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let library = fixture(temp.path())?;
    let mut app = start(temp.path());
    wait(&app, |e| matches!(e, Event::Library(_)))?;
    app.tx.send(Command::SkipSilence(true))?;
    wait(&app, |e| matches!(e, Event::Settings(s) if s.skip_silence))?;
    app.tx.send(Command::Resume(library.books[0].id))?;
    assert!(ready(&app, 0, true)?.skip_silence);
    app.tx.send(Command::Playing(false))?;
    ready(&app, 0, false)?;
    app.tx.send(Command::Next(true))?;
    assert!(ready(&app, 0, false)?.skip_silence);
    app.tx.send(Command::SeekAbsolute(5000))?;
    assert!(ready(&app, 5000, false)?.skip_silence);
    quit(&mut app)?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert!(saved.session.position.abs_diff(5000) < 100);
    let (settings, warning, writable) = Settings::load(&temp.path().join("settings.json"));
    assert!(warning.is_none() && writable && settings.skip_silence);
    let mut restored = start_with_settings(temp.path(), settings);
    assert!(ready(&restored, 5000, false)?.skip_silence);
    restored.tx.send(Command::SkipSilence(false))?;
    wait(
        &restored,
        |e| matches!(e, Event::Settings(s) if !s.skip_silence),
    )?;
    assert!(!ready(&restored, 5000, false)?.skip_silence);
    quit(&mut restored)?;
    assert!(
        !Settings::load(&temp.path().join("settings.json"))
            .0
            .skip_silence
    );
    Ok(())
}
