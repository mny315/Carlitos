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
fn book_edit_acknowledges_its_submitted_context_only_after_commit() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let book = app.library.books[0].id;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    let edit = BookEdit {
        owner: "18446744073709551614".into(),
        title: "  Submitted title  ".into(),
        author: "  Submitted author  ".into(),
        cover: Some("submitted-cover.png".into()),
    };
    app.handle(Command::Edit(book, edit.clone(), vec![]))?;
    let mut acknowledgements = Vec::new();
    while let Ok(event) = received.try_recv() {
        if let Event::BookEdited(id, submitted) = event {
            acknowledgements.push((id, submitted));
        }
    }
    assert_eq!(acknowledgements, [(book, edit.clone())]);
    let saved = app.store()?.load()?;
    assert_eq!(saved.books[0].title, "Submitted title");
    assert_eq!(saved.books[0].author, "Submitted author");
    assert_eq!(saved.books[0].cover, edit.cover);
    rusqlite::Connection::open(temp.path().join("db"))?.execute_batch(
        "CREATE TRIGGER fail_edit BEFORE UPDATE ON books BEGIN SELECT RAISE(ABORT, 'test edit failure'); END;",
    )?;
    let rejected = BookEdit {
        title: "Rejected title".into(),
        ..edit
    };
    assert!(app.handle(Command::Edit(book, rejected, vec![])).is_err());
    while let Ok(event) = received.try_recv() {
        assert!(!matches!(event, Event::BookEdited(..)));
    }
    assert_eq!(app.store()?.load()?.books[0].title, "Submitted title");
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
        BookEdit {
            title: "New title".into(),
            author: "New author".into(),
            cover: Some("new-cover.png".into()),
            ..Default::default()
        },
        order.clone(),
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

#[test]
fn a_failed_book_switch_keeps_the_current_speed_request_for_retry() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _commands) = controller(temp.path())?;
    let book = app.library.books[0].id;
    let current = app.library.book_parts(book)[0].id;
    let other = import_extra_book(&mut app, temp.path(), "other-speed")?;
    let next = app.library.book_parts(other)[0].id;
    app.store()?.save_book_rate(other, 2.0)?;
    app.library.session.current = Some(Target::Book(current));
    app.library.update_progress(3_000, false);
    app.save()?;
    app.library.update_progress(4_000, false);
    app.requested_rate = 1.5;
    app.pending_rate = Some((book, 1.5));
    app.playback = Playback {
        phase: crate::player::Phase::Ready,
        playing: true,
        position: 4_000,
        rate: 1.5,
        ..Default::default()
    };
    let connection = rusqlite::Connection::open(temp.path().join("db"))?;
    connection.execute_batch(
        "CREATE TRIGGER fail_switch BEFORE UPDATE ON session_state BEGIN SELECT RAISE(ABORT, 'test switch failure'); END;",
    )?;
    assert!(app.handle(Command::Part(next, 0)).is_err());
    assert_eq!(app.library.session.current, Some(Target::Book(current)));
    assert_eq!(app.library.session.position, 4_000);
    assert_eq!(app.token, 0);
    assert_eq!(app.requested_rate, 1.5);
    assert_eq!(app.pending_rate, Some((book, 1.5)));
    connection.execute_batch("DROP TRIGGER fail_switch")?;
    app.handle(Command::Playback(app.playback.clone()))?;
    assert_eq!(app.store()?.book_rate(book)?, Some(1.5));
    app.handle(Command::Part(next, 0))?;
    assert_eq!(app.library.session.current, Some(Target::Book(next)));
    assert_eq!(app.requested_rate, 2.0);
    Ok(())
}

fn import_extra_book(app: &mut Controller, root: &Path, name: &str) -> Result<Id> {
    let folder = root.join(name);
    std::fs::create_dir(&folder)?;
    fixture(&folder)?;
    let scan = import::scan(
        vec![folder.join("Книга с пробелами")],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    let before: Vec<_> = app.library.books.iter().map(|b| b.id).collect();
    app.library = app.store()?.import(scan.drafts)?;
    Ok(app
        .library
        .books
        .iter()
        .find(|b| !before.contains(&b.id))
        .unwrap()
        .id)
}

#[test]
fn batch_progress_changes_are_atomic_and_preserve_other_playback() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _) = controller(temp.path())?;
    let first = app.library.books[0].id;
    let second = import_extra_book(&mut app, temp.path(), "second")?;
    let other = import_extra_book(&mut app, temp.path(), "other")?;
    let part = app.library.book_parts(other)[0].id;
    app.library.session.current = Some(Target::Book(part));
    app.library.update_progress(7_000, false);
    app.playback.playing = true;
    app.save()?;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    let completions = || {
        std::iter::from_fn(|| received.try_recv().ok())
            .filter_map(|event| match event {
                Event::BooksChanged(generation) => Some(generation),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let conn = rusqlite::Connection::open(temp.path().join("db"))?;
    conn.execute_batch(&format!("CREATE TRIGGER fail_batch BEFORE INSERT ON book_progress WHEN NEW.book_id={second} AND NEW.completed=1 BEGIN SELECT RAISE(ABORT, 'failure'); END;"))?;
    assert!(
        app.handle(Command::CompleteBooks(vec![first, second], 0))
            .is_err()
    );
    assert_eq!(app.library.progress.len(), 1);
    assert_eq!(app.store()?.load()?.progress.len(), 1);
    assert!(app.playback.playing);
    assert!(
        completions().is_empty(),
        "failed batch must not acknowledge completion"
    );
    conn.execute_batch("DROP TRIGGER fail_batch;")?;
    app.handle(Command::CompleteBooks(vec![first, second], 7))?;
    assert_eq!(completions(), [7]);
    assert!(app.playback.playing);
    assert_eq!(app.library.session.current, Some(Target::Book(part)));
    let saved = app.store()?.load()?;
    assert_eq!(saved.progress.iter().filter(|p| p.completed).count(), 2);
    assert_eq!(
        saved
            .progress
            .iter()
            .find(|p| p.book_id == other)
            .unwrap()
            .position,
        7_000
    );
    conn.execute_batch(&format!("CREATE TRIGGER fail_batch BEFORE DELETE ON book_progress WHEN OLD.book_id={second} BEGIN SELECT RAISE(ABORT, 'failure'); END;"))?;
    assert!(
        app.handle(Command::ResetBooksProgress(vec![first, second], 0))
            .is_err()
    );
    assert_eq!(app.store()?.load()?.progress.len(), 3);
    assert_eq!(app.library.progress.len(), 3);
    assert!(completions().is_empty());
    conn.execute_batch("DROP TRIGGER fail_batch;")?;
    app.handle(Command::ResetBooksProgress(vec![first, second], 9))?;
    assert_eq!(completions(), [9]);
    assert!(app.playback.playing);
    let saved = app.store()?.load()?;
    assert_eq!(saved.progress.len(), 1);
    assert_eq!(saved.progress[0].book_id, other);
    assert_eq!(saved.session.current, Some(Target::Book(part)));
    Ok(())
}

#[test]
fn batch_removal_keeps_audio_by_default_and_deletes_only_selected_audio_when_requested()
-> Result<()> {
    for delete_files in [false, true] {
        let temp = tempfile::tempdir()?;
        let (mut app, _) = controller(temp.path())?;
        let first = app.library.books[0].id;
        let second = import_extra_book(&mut app, temp.path(), "second")?;
        let other = import_extra_book(&mut app, temp.path(), "other")?;
        let files: Vec<_> = app
            .library
            .media
            .iter()
            .map(|m| (m.id, local_path(&m.uri).unwrap()))
            .collect();
        let selected: Vec<_> = app
            .library
            .parts
            .iter()
            .filter(|p| p.book_id != other)
            .map(|p| p.file_id)
            .collect();
        let part = app.library.book_parts(first)[0].id;
        app.library.session.current = Some(Target::Book(part));
        app.library.update_progress(4_000, false);
        app.playback.playing = true;
        let stale = app.playback.clone();
        app.handle(Command::RemoveBooks(vec![first, second], delete_files, 0))?;
        app.handle(Command::Playback(stale))?;
        app.save()?;
        let saved = app.store()?.load()?;
        assert_eq!(saved.books.len(), 1);
        assert_eq!(saved.books[0].id, other);
        assert!(saved.progress.is_empty());
        assert!(saved.session.current.is_none());
        assert!(!app.playback.playing);
        for (id, path) in files {
            assert_eq!(path.exists(), !delete_files || !selected.contains(&id));
        }
    }
    Ok(())
}

#[test]
fn batch_record_removal_rolls_back_and_shared_audio_cannot_be_deleted() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _) = controller(temp.path())?;
    let first = app.library.books[0].id;
    let second = import_extra_book(&mut app, temp.path(), "second")?;
    let conn = rusqlite::Connection::open(temp.path().join("db"))?;
    conn.execute_batch(&format!("CREATE TRIGGER fail_batch BEFORE DELETE ON books WHEN OLD.id={second} BEGIN SELECT RAISE(ABORT, 'failure'); END;"))?;
    assert!(
        app.handle(Command::RemoveBooks(vec![first, second], false, 0))
            .is_err()
    );
    assert!(
        app.handle(Command::RemoveBooks(vec![first, second], true, 0))
            .is_err()
    );
    assert!(
        app.library
            .media
            .iter()
            .all(|m| local_path(&m.uri).unwrap().exists())
    );
    assert_eq!(app.library.books.len(), 2);
    assert_eq!(app.store()?.load()?.books.len(), 2);
    conn.execute_batch("DROP TRIGGER fail_batch;")?;
    let shared = app.library.book_parts(first)[0].file_id;
    conn.execute(
        "INSERT INTO book_parts(book_id,file_id,ordinal,title) VALUES (?1,?2,3,'Shared')",
        rusqlite::params![second, shared],
    )?;
    app.library = app.store()?.load()?;
    let path = local_path(&app.library.media(shared).unwrap().uri).unwrap();
    assert!(
        app.handle(Command::RemoveBooks(vec![first], true, 0))
            .is_err()
    );
    assert!(path.exists());
    assert_eq!(app.store()?.load()?.books.len(), 2);
    Ok(())
}

#[test]
fn deleting_books_rejects_replaced_audio_before_removing_any_file() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _) = controller(temp.path())?;
    let book = app.library.books[0].id;
    let files: Vec<_> = app
        .library
        .book_parts(book)
        .iter()
        .map(|p| local_path(&app.library.media(p.file_id).unwrap().uri).unwrap())
        .collect();
    let last = files.last().unwrap();
    // The saved book refers to the original inode, not a new recording which
    // has since been placed at the same path with the same size and timestamp.
    std::fs::rename(last, last.with_extension("original"))?;
    std::fs::copy(last.with_extension("original"), last)?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(last)?
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::fs::metadata(last.with_extension("original"))?.modified()?),
        )?;
    assert!(
        app.handle(Command::RemoveBooks(vec![book], true, 0))
            .is_err()
    );
    assert!(files.iter().all(|path| path.is_file()));
    assert_eq!(app.store()?.load()?.books.len(), 1);
    // Removing just the stale library entry remains possible.
    app.handle(Command::RemoveBooks(vec![book], false, 0))?;
    assert!(files.iter().all(|path| path.is_file()));
    assert!(app.store()?.load()?.books.is_empty());
    Ok(())
}

#[test]
fn delayed_removal_cannot_target_a_book_imported_after_the_original_was_removed() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _) = controller(temp.path())?;
    let old = app.library.books[0].id;
    app.handle(Command::RemoveBook(old))?;
    let new = import_extra_book(&mut app, temp.path(), "replacement")?;
    let files: Vec<_> = app
        .library
        .media
        .iter()
        .map(|m| local_path(&m.uri).unwrap())
        .collect();
    // A UI event from the old library snapshot can already be queued when an
    // independent picker/scan result imports another book on the worker.
    app.handle(Command::RemoveBooks(vec![old], true, 0))?;
    assert_eq!(app.library.books.len(), 1);
    assert_eq!(app.library.books[0].id, new);
    assert!(files.iter().all(|path| path.is_file()));
    Ok(())
}
