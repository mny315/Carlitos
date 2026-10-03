use super::*;

#[test]
fn a_failed_completion_keeps_playback_and_progress_unchanged() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let book = app.library.books[0].id;
    let part = app.library.parts[0].id;
    app.library.session.current = Some(Target::Book(part));
    app.library.update_progress(5_000, false);
    app.playback = Playback {
        position: 5_000,
        duration: Some(12_000),
        playing: true,
        phase: crate::player::Phase::Ready,
        ..Default::default()
    };
    let stale = app.playback.clone();
    app.save()?;
    rusqlite::Connection::open(temp.path().join("db"))?.execute_batch(
        "CREATE TRIGGER fail_complete BEFORE INSERT ON book_progress WHEN NEW.completed=1 BEGIN SELECT RAISE(ABORT, 'test completion failure'); END;",
    )?;
    assert!(app.handle(Command::Complete(book)).is_err());
    assert!(app.playback.playing);
    assert_eq!(app.token, 0);
    assert_eq!(app.library.session.current, Some(Target::Book(part)));
    assert_eq!(app.library.progress[0].position, 5_000);
    assert!(!app.library.progress[0].completed);
    // A later checkpoint must not silently commit the rejected operation.
    app.save()?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.current, Some(Target::Book(part)));
    assert_eq!(saved.progress[0].position, 5_000);
    assert!(!saved.progress[0].completed);
    rusqlite::Connection::open(temp.path().join("db"))?
        .execute_batch("DROP TRIGGER fail_complete;")?;
    app.handle(Command::Complete(book))?;
    app.handle(Command::Playback(stale))?;
    app.save()?;
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert!(!app.playback.playing);
    assert!(saved.session.current.is_none());
    assert!(saved.progress[0].completed);
    assert_eq!(saved.progress[0].part_id, saved.book_parts(book)[2].id);
    assert_eq!(saved.progress[0].position, 12_000);
    Ok(())
}

#[test]
fn resetting_a_book_persists_and_rejects_old_playback_snapshots() -> Result<()> {
    for completed in [false, true] {
        let temp = tempfile::tempdir()?;
        let (mut app, _commands) = controller(temp.path())?;
        let book = app.library.books[0].id;
        app.library.session.current = Some(Target::Book(app.library.parts[1].id));
        app.library.update_progress(5_000, completed);
        app.playback = Playback {
            position: 5_000,
            duration: Some(12_000),
            playing: true,
            phase: crate::player::Phase::Ready,
            ..Default::default()
        };
        let stale = app.playback.clone();
        app.handle(Command::ResetProgress(book))?;
        assert!(app.library.progress.is_empty());
        assert!(app.library.session.current.is_none());
        assert!(!app.playback.playing);
        app.handle(Command::Playback(stale))?;
        app.save()?;
        let saved = Store::open(&temp.path().join("db"))?.load()?;
        assert!(saved.progress.is_empty());
        assert!(saved.session.current.is_none());
        assert_eq!(saved.books.len(), 1);
        assert_eq!(saved.parts.len(), 3);
        assert_eq!(saved.media.len(), 3);
        app.handle(Command::Resume(book))?;
        assert_eq!(
            app.library.session.current,
            Some(Target::Book(saved.book_parts(book)[0].id))
        );
        assert_eq!(app.library.session.position, 0);
    }
    Ok(())
}

#[test]
fn resetting_another_book_keeps_current_playback_and_progress() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let book = app.library.books[0].id;
    app.library.session.current = Some(Target::Book(app.library.parts[0].id));
    app.library.update_progress(3_000, true);
    app.save()?;
    let other = temp.path().join("other");
    std::fs::create_dir(&other)?;
    fixture(&other)?;
    let scan = import::scan(
        vec![other.join("Книга с пробелами")],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    app.library = app.store()?.import(scan.drafts)?;
    let other_book = app.library.books.iter().find(|b| b.id != book).unwrap().id;
    let part = app.library.book_parts(other_book)[0].id;
    app.library.session.current = Some(Target::Book(part));
    app.library.update_progress(7_000, false);
    app.playback.playing = true;
    let token = app.token;
    app.handle(Command::ResetProgress(book))?;
    assert!(app.playback.playing);
    assert_eq!(app.token, token);
    assert_eq!(app.library.session.current, Some(Target::Book(part)));
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.current, Some(Target::Book(part)));
    assert_eq!(saved.progress.len(), 1);
    assert_eq!(saved.progress[0].book_id, other_book);
    assert_eq!(saved.progress[0].position, 7_000);
    Ok(())
}

#[test]
fn a_failed_progress_reset_keeps_the_book_and_player_unchanged() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let book = app.library.books[0].id;
    let part = app.library.parts[0].id;
    app.library.session.current = Some(Target::Book(part));
    app.library.update_progress(5_000, false);
    app.playback.playing = true;
    app.save()?;
    rusqlite::Connection::open(temp.path().join("db"))?.execute_batch(
        "CREATE TRIGGER fail_reset BEFORE DELETE ON book_progress BEGIN SELECT RAISE(ABORT, 'test reset failure'); END;"
    )?;
    assert!(app.handle(Command::ResetProgress(book)).is_err());
    assert!(app.playback.playing);
    assert_eq!(app.token, 0);
    assert_eq!(app.library.session.current, Some(Target::Book(part)));
    assert_eq!(app.library.progress[0].position, 5_000);
    let saved = Store::open(&temp.path().join("db"))?.load()?;
    assert_eq!(saved.session.current, Some(Target::Book(part)));
    assert_eq!(saved.progress[0].position, 5_000);
    Ok(())
}

#[test]
fn queued_part_moves_preserve_latest_book_edits_and_progress() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let book = app.library.books[0].id;
    let order: Vec<_> = app.library.book_parts(book).iter().map(|p| p.id).collect();
    app.library.session.current = Some(Target::Book(order[0]));
    app.library.update_progress(4_000, false);
    app.handle(Command::Edit(
        book,
        "New title".into(),
        "New author".into(),
        order.clone(),
        Some("new-cover.png".into()),
    ))?;
    app.handle(Command::MovePart(order[2], false))?;
    app.handle(Command::MovePart(order[2], false))?;
    let saved = app.store()?.load()?;
    let actual: Vec<_> = saved.book_parts(book).iter().map(|p| p.id).collect();
    assert_eq!(actual, [order[2], order[0], order[1]]);
    assert_eq!(saved.books[0].title, "New title");
    assert_eq!(saved.books[0].author, "New author");
    assert_eq!(saved.books[0].cover.as_deref(), Some("new-cover.png"));
    assert_eq!(saved.progress[0].part_id, order[0]);
    assert_eq!(saved.progress[0].position, 4_000);
    Ok(())
}
