use super::*;
use crate::{
    library::*,
    storage::{Draft, Store},
};
use std::{path::Path, sync::atomic::AtomicBool};

fn fixture(root: &Path, name: &str, content: &[u8]) -> Result<(Store, Library)> {
    let directory = root.join(name);
    std::fs::create_dir_all(&directory)?;
    let path = directory.join("renamed.mp3");
    std::fs::write(&path, content)?;
    let mut store = Store::open(&directory.join("db"))?;
    let library = store.import(vec![Draft {
        root_uri: file_uri(&directory)?,
        title: name.into(),
        author: "Author".into(),
        include: true,
        files: vec![Media {
            uri: file_uri(&path)?,
            identity: name.into(),
            relative: "renamed.mp3".into(),
            size: Some(content.len() as u64),
            modified: Some(1),
            duration: Some(120_000),
            ..Default::default()
        }],
    }])?;
    let hash = Fingerprint::read(&library.media[0], &AtomicBool::new(false))?;
    store.save_fingerprints(&library, &[hash])?;
    Ok((store, library))
}
fn progress(store: &mut Store, library: &mut Library, position: u64) -> Result<Vec<Record>> {
    library.session.current = Some(Target::Book(library.parts[0].id));
    library.update_progress(position, false);
    store.save(&library.session, &library.progress)?;
    store.sync_records(library)
}

#[test]
fn book_speed_sync_is_independent_of_progress_and_survives_restart() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut a, mut first) = fixture(temp.path(), "computer", b"same recording")?;
    let (mut b, second) = fixture(temp.path(), "phone", b"same recording")?;
    let book = first.books[0].id;
    a.save_book_rate(book, 1.75)?;
    let rate = a.sync_rates(&first)?;
    assert!(a.sync_records(&first)?.is_empty());
    assert_eq!(
        b.merge_sync_rates(&second, &rate)?,
        vec![second.books[0].id]
    );
    assert!(b.load()?.progress.is_empty());
    assert!(b.merge_sync_rates(&second, &rate)?.is_empty());
    let progress = progress(&mut a, &mut first, 90_000)?;
    let (second, _) = b.merge_sync(&second, &progress)?;
    b.save_book_rate(second.books[0].id, 2.0)?;
    let newer = b.sync_rates(&second)?;
    assert!(newer[0].revision() > rate[0].revision());
    a.merge_sync_rates(&first, &newer)?;
    assert_eq!(a.sync_records(&first)?, progress);
    assert_eq!(a.sync_rates(&first)?, newer);
    assert!(a.merge_sync_rates(&first, &rate)?.is_empty());
    a.save_book_rate(book, 2.0)?;
    assert_eq!(
        a.sync_rates(&first)?,
        newer,
        "Acknowledging remote speed must not create a new revision"
    );
    a.reset_book_progress(book)?;
    assert_eq!(a.book_rate(book)?, Some(2.0));
    drop(a);
    let a = Store::open(&temp.path().join("computer/db"))?;
    assert_eq!(a.book_rate(book)?, Some(2.0));
    Ok(())
}

#[test]
fn speed_conflicts_converge_and_invalid_or_unmatched_values_do_not_apply() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut store, library) = fixture(temp.path(), "a", b"edition")?;
    let book = library.books[0].id;
    store.save_book_rate(book, 1.5)?;
    let mut rates = store.sync_rates(&library)?;
    rates[0].updated += 60_000;
    rates[0].device = "ff".repeat(16);
    rates[0].value = 2.0;
    assert_eq!(store.merge_sync_rates(&library, &rates)?, vec![book]);
    let mut losing = rates.clone();
    losing[0].device = "00".repeat(16);
    losing[0].value = 1.0;
    assert!(store.merge_sync_rates(&library, &losing)?.is_empty());
    store.save_book_rate(book, 1.25)?;
    let local = store.sync_rates(&library)?;
    assert!(local[0].revision() > rates[0].revision());
    for value in [f64::NAN, f64::INFINITY, 0.0, 3.01] {
        rates[0].value = value;
        assert!(store.merge_sync_rates(&library, &rates).is_err());
    }
    rates[0].value = 2.0;
    rates[0].book = "ee".repeat(32);
    assert!(store.merge_sync_rates(&library, &rates)?.is_empty());
    assert_eq!(store.sync_rates(&library)?, local);
    Ok(())
}

#[test]
fn latest_change_wins_even_after_rewind_and_does_not_bounce_between_devices() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut a, mut first) = fixture(temp.path(), "computer", b"same recording")?;
    let (mut b, mut second) = fixture(temp.path(), "phone", b"same recording")?;
    let original = progress(&mut a, &mut first, 90_000)?;
    let (received, changed) = b.merge_sync(&second, &original)?;
    second = received;
    assert_eq!(changed.len(), 1);
    assert_eq!(second.progress[0].position, 90_000);
    assert_eq!(b.sync_records(&second)?, original);
    assert!(
        a.merge_sync(&first, &b.sync_records(&second)?)?
            .1
            .is_empty()
    );
    let rewind = progress(&mut b, &mut second, 20_000)?;
    assert!(rewind[0].revision() > original[0].revision());
    let (received, changed) = a.merge_sync(&first, &rewind)?;
    first = received;
    assert_eq!(changed.len(), 1);
    assert_eq!(first.session.position, 20_000);
    assert_eq!(first.progress[0].position, 20_000);
    assert_eq!(a.sync_records(&first)?, rewind);
    assert!(a.merge_sync(&first, &original)?.1.is_empty());
    // Repeated paused/restore snapshots are not new listening activity.
    first.update_progress(20_000, false);
    a.save(&first.session, &first.progress)?;
    assert_eq!(a.sync_records(&first)?, rewind);
    Ok(())
}

#[test]
fn reset_is_a_durable_change_and_older_progress_cannot_resurrect_it() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut a, mut first) = fixture(temp.path(), "a", b"recording")?;
    let (mut b, second) = fixture(temp.path(), "b", b"recording")?;
    let old = progress(&mut a, &mut first, 30_000)?;
    let (second, _) = b.merge_sync(&second, &old)?;
    b.reset_book_progress(second.books[0].id)?;
    drop(b);
    let b = Store::open(&temp.path().join("b/db"))?;
    let reset = b.sync_records(&b.load()?)?;
    assert!(reset[0].reset);
    let (first, _) = a.merge_sync(&first, &reset)?;
    assert!(first.progress.is_empty());
    assert!(first.session.current.is_none());
    assert!(a.merge_sync(&first, &old)?.1.is_empty());
    assert_eq!(a.sync_records(&a.load()?)?, reset);
    Ok(())
}

#[test]
fn same_title_different_recording_does_not_match_and_bad_dates_are_rejected() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let (mut a, mut first) = fixture(temp.path(), "a", b"edition one")?;
    let (mut b, mut second) = fixture(temp.path(), "b", b"edition two")?;
    second.books[0].title = first.books[0].title.clone();
    let mut record = progress(&mut a, &mut first, 10_000)?;
    assert!(b.merge_sync(&second, &record)?.1.is_empty());
    record[0].updated = i64::MAX;
    assert!(b.merge_sync(&second, &record).is_err());
    assert!(b.load()?.progress.is_empty());
    Ok(())
}

#[test]
fn equal_timestamps_converge_by_origin_and_clock_rollback_cannot_undo_a_local_change() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let (mut a, mut library) = fixture(temp.path(), "a", b"edition")?;
    let mut record = progress(&mut a, &mut library, 90_000)?;
    record[0].updated += 60_000;
    record[0].device = "ff".repeat(16);
    record[0].position = 40_000;
    let (updated, changed) = a.merge_sync(&library, &record)?;
    library = updated;
    assert_eq!(changed.len(), 1);
    let mut losing = record.clone();
    losing[0].device = "00".repeat(16);
    losing[0].position = 5_000;
    assert!(a.merge_sync(&library, &losing)?.1.is_empty());
    let local = progress(&mut a, &mut library, 15_000)?;
    assert!(local[0].revision() > record[0].revision());
    assert_eq!(local[0].device, a.sync_device()?);
    Ok(())
}

#[test]
fn full_content_hash_detects_changes_in_the_middle_and_cache_checks_file_signature() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let bytes = vec![42; 900_000];
    let (store, mut library) = fixture(temp.path(), "a", &bytes)?;
    let original = store.fingerprints()?;
    assert_eq!(Index::new(&library, &original).books.len(), 1);
    library.media[0].modified = Some(2);
    assert!(Index::new(&library, &original).books.is_empty());
    let mut changed = bytes;
    changed[400_000] = 43;
    std::fs::write(local_path(&library.media[0].uri).unwrap(), changed)?;
    let hash = Fingerprint::read(&library.media[0], &AtomicBool::new(false))?;
    assert_ne!(original[0].hash, hash.hash);
    assert!(Fingerprint::read(&library.media[0], &AtomicBool::new(true)).is_err());
    Ok(())
}

#[test]
#[ignore = "requires the isolated Android test APK; see tests/README.md"]
fn android_qr_and_progress_roundtrip() -> Result<()> {
    use std::{
        sync::mpsc,
        time::{Duration, Instant},
    };
    let root = std::path::PathBuf::from(std::env::var("CARLITOS_SYNC_DEVICE_DIR")?);
    std::fs::create_dir_all(&root)?;
    let temp = tempfile::tempdir_in(&root)?;
    let samples = 120 * 8_000u32;
    let size = samples * 2;
    let mut audio = Vec::new();
    audio.extend_from_slice(b"RIFF");
    audio.extend_from_slice(&(36 + size).to_le_bytes());
    audio.extend_from_slice(b"WAVEfmt ");
    audio.extend_from_slice(&16u32.to_le_bytes());
    audio.extend_from_slice(&1u16.to_le_bytes());
    audio.extend_from_slice(&1u16.to_le_bytes());
    audio.extend_from_slice(&8000u32.to_le_bytes());
    audio.extend_from_slice(&16000u32.to_le_bytes());
    audio.extend_from_slice(&2u16.to_le_bytes());
    audio.extend_from_slice(&16u16.to_le_bytes());
    audio.extend_from_slice(b"data");
    audio.extend_from_slice(&size.to_le_bytes());
    audio.resize(44 + size as usize, 0);
    std::fs::write(root.join("book.wav"), &audio)?;
    let (mut store, mut library) = fixture(temp.path(), "desktop", &audio)?;
    let records = progress(&mut store, &mut library, 90_000)?;
    store.save_book_rate(library.books[0].id, 1.75)?;
    let (tx, rx) = mpsc::channel();
    let network = Network::start(temp.path().join("devices.json"), store.sync_device()?, tx)?;
    network.publish(records, store.sync_rates(&library)?);
    network.request(Request::Invite);
    let until = Instant::now() + Duration::from_secs(180);
    let mut rewind = false;
    let mut speed_received = false;
    let mut ready = false;
    loop {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now()))? {
            crate::app::Command::SyncStatus(status) if !ready && !status.invitation.is_empty() => {
                let pixels = qr_image(&status.invitation)?.to_rgb8().unwrap();
                image::save_buffer(
                    root.join("pair.png"),
                    pixels.as_bytes(),
                    pixels.width(),
                    pixels.height(),
                    image::ColorType::Rgb8,
                )?;
                std::fs::write(root.join("ready"), "ready")?;
                ready = true;
                println!("Desktop sync peer ready; generated test audio and pairing QR");
            }
            crate::app::Command::SyncIncoming(records, rates) => {
                store.merge_sync_rates(&library, &rates)?;
                let (received, _) = store.merge_sync(&library, &records)?;
                library = received;
                let records = store.sync_records(&library)?;
                network.publish(records.clone(), store.sync_rates(&library)?);
                speed_received |= store.book_rate(library.books[0].id)? == Some(2.25);
                rewind |= records.iter().any(|r| r.position == 20_000 && !r.reset);
                if rewind && speed_received && records.iter().any(|r| r.reset) {
                    assert!(store.load()?.progress.is_empty());
                    println!(
                        "PASS Android to desktop book speed, rewind and durable reset over the local network"
                    );
                    break;
                }
            }
            _ => {}
        }
    }
    Ok(())
}
