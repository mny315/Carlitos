use super::*;
use anyhow::Context;

#[test]
fn sync_ignores_missing_media_from_removed_books() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, commands) = controller(temp.path())?;
    let mut orphan = app.library.media[0].clone();
    orphan.id = 999;
    orphan.uri = url::Url::from_file_path(temp.path().join("deleted.wav"))
        .unwrap()
        .into();
    app.library.media.push(orphan);
    app.options.settings.local_sync = true;
    app.configure_sync()?;
    let mut hashed = 0;
    loop {
        match commands.recv_timeout(Duration::from_secs(10))? {
            Command::SyncFingerprints(files, done) => {
                assert!(files.iter().all(|f| f.file != 999));
                hashed += files.len();
                if done {
                    break;
                }
            }
            Command::Error(message) => bail!("Unused file triggered sync error: {message}"),
            _ => {}
        }
    }
    assert_eq!(hashed, app.library.media.len() - 1);
    Ok(())
}

#[test]
fn sync_read_error_names_the_file_and_preserves_the_cause() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, commands) = controller(temp.path())?;
    let file = &app.library.media[0];
    let name = file.relative.clone();
    std::fs::remove_file(crate::library::local_path(&file.uri).unwrap())?;
    app.options.settings.local_sync = true;
    app.configure_sync()?;
    let mut failure = None;
    loop {
        match commands.recv_timeout(Duration::from_secs(10))? {
            Command::Error(message) => failure = Some(message),
            Command::SyncFingerprints(_, true) => break,
            _ => {}
        }
    }
    let message = failure.context("Missing file did not report a sync error")?;
    assert!(message.contains(&name), "{message}");
    assert!(message.contains("os error"), "{message}");
    Ok(())
}

#[test]
fn fingerprinted_books_are_not_marked_ready_without_a_matching_remote_inventory() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, commands) = controller(temp.path())?;
    let (events, received) = async_channel::unbounded();
    app.events = events;
    app.options.settings.local_sync = true;
    app.configure_sync()?;
    let library = app.library.clone();
    let fingerprints = library
        .media
        .iter()
        .map(|file| {
            crate::sync::Fingerprint::read(file, &std::sync::atomic::AtomicBool::new(false))
        })
        .collect::<Result<Vec<_>>>()?;
    app.store()?.save_fingerprints(&library, &fingerprints)?;
    app.publish_sync();
    loop {
        let command = commands.recv_timeout(Duration::from_secs(5))?;
        if let Command::SyncStatus(status) = command {
            assert!(status.ready_books.is_empty());
            app.handle(Command::SyncStatus(status))?;
            break;
        }
    }
    assert!(
        std::iter::from_fn(|| received.try_recv().ok())
            .all(|event| !matches!(event, Event::SyncReady(books) if !books.is_empty()))
    );
    app.handle(Command::SyncStatus(crate::sync::Status {
        ready_books: vec![library.books[0].id],
        ..Default::default()
    }))?;
    assert!(
        std::iter::from_fn(|| received.try_recv().ok()).any(
            |event| matches!(event, Event::SyncReady(books) if books == [library.books[0].id])
        )
    );
    app.handle(Command::SyncStatus(crate::sync::Status::default()))?;
    assert!(
        std::iter::from_fn(|| received.try_recv().ok())
            .any(|event| matches!(event, Event::SyncReady(books) if books.is_empty()))
    );
    Ok(())
}

use crate::sync::{Fingerprint, Network};
use std::sync::atomic::AtomicBool;

#[test]
fn fingerprinting_prioritizes_active_then_started_books_including_shared_recordings() -> Result<()>
{
    for shared in [false, true] {
        let temp = tempfile::tempdir()?;
        let (mut app, rx) = controller(temp.path())?;
        let book = app.library.books[0].clone();
        for id in [book.id + 1, book.id + 2] {
            app.library.books.push(Book { id, ..book.clone() });
        }
        for (index, part) in app.library.parts.iter_mut().enumerate() {
            part.book_id = book.id + index as Id;
        }
        app.library.session.current = Some(Target::Book(app.library.parts[0].id));
        app.library.update_progress(1_000, false);
        app.library.session.current = Some(Target::Book(app.library.parts[2].id));
        if shared {
            let mut part = app.library.parts[0].clone();
            part.id = app.library.parts[2].id + 1;
            part.book_id = book.id + 2;
            app.library.parts.push(part);
        }
        let device = app.store()?.sync_device()?;
        app.sync = Some(Network::without_discovery(
            temp.path().join("sync.json"),
            device,
            app.tx.clone(),
        )?);
        app.refresh_sync()?;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut files = Vec::new();
        loop {
            let command = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
            if let Command::SyncFingerprints(fingerprints, done) = command {
                files.extend(fingerprints.into_iter().map(|f| f.file));
                if done {
                    break;
                }
            }
        }
        let media = &app.library.media;
        assert_eq!(
            files,
            if shared {
                vec![media[0].id, media[2].id, media[1].id]
            } else {
                vec![media[2].id, media[0].id, media[1].id]
            }
        );
    }
    Ok(())
}

#[test]
fn a_failed_remote_position_load_cannot_reaccept_the_old_decoders_checkpoint() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut app, _, stale, incoming) = remote_position(temp.path())?;
    // The audio worker can exit after queuing its final Ready snapshot and
    // before the controller tries to load the incoming remote position.
    app.audio.stop();
    assert!(app.receive_sync(incoming.clone(), vec![]).is_err());
    app.handle(Command::Playback(stale))?;
    assert_eq!(app.library.progress[0].position, 9_000);
    app.save()?;
    let library = app.store()?.load()?;
    assert_eq!(library.progress[0].position, 9_000);
    assert_eq!(app.store()?.sync_records(&library)?, incoming);
    Ok(())
}

#[test]
fn a_remote_position_restores_paused_without_revising_progress_or_losing_pending_rate() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let (mut app, rx, stale, incoming) = remote_position(temp.path())?;
    let part = app.library.parts[0].id;
    let book = app.library.parts[0].book_id;
    app.pending_rate = Some((book, 1.5));
    app.receive_sync(incoming.clone(), vec![])?;
    app.handle(Command::Playback(stale))?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let command = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))?;
        app.handle(command)?;
        if app.playback.phase == crate::player::Phase::Ready {
            break;
        }
    }
    assert_eq!(app.library.session.current, Some(Target::Book(part)));
    assert_eq!(app.library.session.position, 9_000);
    assert!(!app.playback.playing);
    assert_eq!(app.playback.position, 9_000);
    assert_eq!(app.playback.rate, 1.5);
    assert_eq!(app.pending_rate, None);
    assert_eq!(app.store()?.book_rate(book)?, Some(1.5));
    app.save()?;
    let library = app.store()?.load()?;
    assert_eq!(app.store()?.sync_records(&library)?, incoming);
    Ok(())
}

fn remote_position(
    root: &Path,
) -> Result<(
    Controller,
    mpsc::Receiver<Command>,
    Playback,
    Vec<crate::sync::Record>,
)> {
    let (mut app, rx) = controller(root)?;
    let part = app.library.parts[0].id;
    app.library.session.current = Some(Target::Book(part));
    app.library.update_progress(3_000, false);
    app.playback = Playback {
        position: 3_000,
        phase: crate::player::Phase::Ready,
        playing: true,
        ..Default::default()
    };
    let stale = app.playback.clone();
    app.save()?;
    let library = app.library.clone();
    let fingerprints = library
        .media
        .iter()
        .map(|m| Fingerprint::read(m, &AtomicBool::new(false)))
        .collect::<Result<Vec<_>>>()?;
    app.store()?.save_fingerprints(&library, &fingerprints)?;
    let mut incoming = app.store()?.sync_records(&library)?;
    incoming[0].position = 9_000;
    incoming[0].updated += 60_000;
    incoming[0].device = "ff".repeat(16);
    let device = app.store()?.sync_device()?;
    app.options.settings.local_sync = true;
    app.sync = Some(Network::without_discovery(
        root.join("sync.json"),
        device,
        app.tx.clone(),
    )?);
    Ok((app, rx, stale, incoming))
}
