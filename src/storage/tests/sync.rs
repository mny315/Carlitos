use super::*;
use crate::sync::{Fingerprint, Index, RateRecord, Record};

#[test]
fn mixed_sync_revisions_keep_progress_order_and_do_not_reselect_a_reset_book() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let mut library = store.import(vec![draft("a", 1), draft("b", 1), draft("c", 1)])?;
    for part in library.parts.clone() {
        library.session.current = Some(Target::Book(part.id));
        library.update_progress(500, false);
    }
    let fingerprints: Vec<_> = library
        .media
        .iter()
        .map(|media| Fingerprint {
            file: media.id,
            signature: Fingerprint::signature(media),
            hash: format!("{:064x}", media.id),
        })
        .collect();
    store.save_fingerprints(&library, &fingerprints)?;
    let index = Index::new(&library, &fingerprints);
    let ids: Vec<_> = library.books.iter().map(|b| b.id).collect();
    library.session.current = Some(Target::Book(index.books[&ids[1]].parts[0].0));
    store.save(&library.session, &library.progress)?;
    let updated = now_millis() + 60_000;
    let record = |id, position, revision, reset| Record {
        book: index.books[&id].key.clone(),
        part: index.books[&id].parts[0].1.clone(),
        occurrence: 0,
        position,
        completed: false,
        reset,
        updated: updated + revision,
        device: "aa".repeat(16),
    };
    let records = [
        record(ids[0], 2_000, 0, false),
        record(ids[1], 0, 0, true),
        record(ids[0], 3_000, 1, false),
        record(ids[1], 4_000, 1, false),
        record(ids[0], 5_000, 0, false),
        record(ids[2], 1_000_000, 0, false),
    ];
    let (library, changed) = store.merge_sync(&library, &records)?;
    assert_eq!(changed, vec![ids[0], ids[1], ids[0], ids[1]]);
    assert!(library.session.current.is_none());
    assert_eq!(library.session.position, 0);
    assert_eq!(
        library
            .progress
            .iter()
            .map(|p| (p.book_id, p.position))
            .collect::<Vec<_>>(),
        vec![(ids[2], 500), (ids[0], 3_000), (ids[1], 4_000)]
    );
    let saved = store.load()?;
    assert!(saved.session.current.is_none());
    assert_eq!(
        saved
            .progress
            .iter()
            .find(|p| p.book_id == ids[1])
            .unwrap()
            .position,
        4_000
    );
    Ok(())
}

#[test]
fn a_large_exchange_updates_matching_copies_and_ignores_replayed_revisions() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let count = crate::sync::MAX_RECORDS as Id;
    let tx = store.conn.transaction()?;
    tx.execute(
        "WITH RECURSIVE ids(id) AS (SELECT 1 UNION ALL SELECT id+1 FROM ids WHERE id<?1)
         INSERT INTO sources SELECT id,'file:///audit/'||id,'book' FROM ids",
        [count],
    )?;
    tx.execute(
        "INSERT INTO media_files SELECT id,id,uri||'/part.wav','part.wav','audio:'||id,0,-1,?1 FROM sources",
        [serde_json::to_string(&Media {
            duration: Some(60_000),
            ..Default::default()
        })?],
    )?;
    tx.execute_batch(
        "INSERT INTO books SELECT id,id,'Book '||id,'',NULL FROM sources;
         INSERT INTO book_parts SELECT id,id,id,0,'Part' FROM sources;",
    )?;
    tx.commit()?;
    let mut library = store.load()?;
    library.session.current = Some(Target::Book(count));
    let fingerprints: Vec<_> = library
        .media
        .iter()
        .map(|media| Fingerprint {
            file: media.id,
            signature: Fingerprint::signature(media),
            // The final book is a second local copy of the first recording.
            hash: format!("{:064x}", if media.id == count { 1 } else { media.id }),
        })
        .collect();
    store.save_fingerprints(&library, &fingerprints)?;
    let index = Index::new(&library, &fingerprints);
    let updated = now_millis();
    let records: Vec<_> = index
        .books
        .iter()
        .rev()
        .filter(|(id, _)| **id != count)
        .map(|(&id, book)| Record {
            book: book.key.clone(),
            part: book.parts[0].1.clone(),
            occurrence: 0,
            position: id as u64 + 1_000,
            completed: false,
            reset: false,
            updated,
            device: "aa".repeat(16),
        })
        .collect();
    let rates: Vec<_> = records
        .iter()
        .map(|record| RateRecord {
            book: record.book.clone(),
            value: 1.75,
            updated,
            device: record.device.clone(),
        })
        .collect();
    let started = std::time::Instant::now();
    let (library, changed, speeds) = store.merge_sync_exchange(&library, &records, &rates)?;
    println!("Merge of {count} books: {:?}", started.elapsed());
    assert_eq!(changed.len(), count as usize);
    assert_eq!(speeds.len(), count as usize);
    assert_eq!(library.progress.len(), count as usize);
    assert_eq!(library.session.current, Some(Target::Book(count)));
    assert_eq!(library.session.position, 1_001);
    for progress in &library.progress {
        let original = if progress.book_id == count {
            1
        } else {
            progress.book_id
        };
        assert_eq!(progress.position, original as u64 + 1_000);
        assert_eq!(store.book_rate(progress.book_id)?, Some(1.75));
    }
    assert_eq!(store.load()?.progress.len(), count as usize);
    let started = std::time::Instant::now();
    let (replayed, changed, speeds) = store.merge_sync_exchange(&library, &records, &rates)?;
    println!("Replay of {count} books: {:?}", started.elapsed());
    assert!(changed.is_empty());
    assert!(speeds.is_empty());
    assert_eq!(replayed.session.position, library.session.position);
    assert_eq!(
        replayed
            .progress
            .iter()
            .map(|p| p.book_id)
            .collect::<Vec<_>>(),
        library
            .progress
            .iter()
            .map(|p| p.book_id)
            .collect::<Vec<_>>()
    );
    Ok(())
}
