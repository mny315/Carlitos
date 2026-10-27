use super::*;
use crate::sync::{Fingerprint, Index, RateRecord, Record};
use rusqlite::TransactionBehavior;
use std::collections::BTreeMap;

pub(super) fn migrate(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    anyhow::ensure!(version <= 8, "Library schema changed during migration");
    if version >= 6 {
        return Ok(());
    }
    tx.execute_batch(
        "CREATE TABLE sync_identity(id TEXT NOT NULL);
         CREATE TABLE sync_fingerprints(file_id INTEGER PRIMARY KEY REFERENCES media_files(id) ON DELETE CASCADE,signature TEXT NOT NULL,hash TEXT NOT NULL);
         CREATE TABLE sync_changes(book_id INTEGER PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,updated INTEGER NOT NULL,device TEXT NOT NULL,reset INTEGER NOT NULL);
         UPDATE book_progress SET updated=updated*1000;"
    )?;
    tx.execute(
        "INSERT INTO sync_identity VALUES(?1)",
        [crate::sync::random_hex(16)?],
    )?;
    tx.execute_batch(
        "INSERT INTO sync_changes SELECT book_id,updated,(SELECT id FROM sync_identity),0 FROM book_progress;
         CREATE TRIGGER sync_progress_insert AFTER INSERT ON book_progress BEGIN
           INSERT INTO sync_changes VALUES(NEW.book_id,NEW.updated,(SELECT id FROM sync_identity),0)
           ON CONFLICT(book_id) DO UPDATE SET updated=MAX(excluded.updated,sync_changes.updated+1),device=excluded.device,reset=0;
         END;
         CREATE TRIGGER sync_progress_update AFTER UPDATE ON book_progress BEGIN
           INSERT INTO sync_changes VALUES(NEW.book_id,NEW.updated,(SELECT id FROM sync_identity),0)
           ON CONFLICT(book_id) DO UPDATE SET updated=MAX(excluded.updated,sync_changes.updated+1),device=excluded.device,reset=0;
         END;"
    )?;
    tx.pragma_update(None, "user_version", 6)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn migrate_rates(conn: &mut Connection) -> Result<()> {
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    anyhow::ensure!(version <= 8, "Library schema changed during migration");
    if version >= 7 {
        return Ok(());
    }
    anyhow::ensure!(version == 6, "Expected library schema 6");
    tx.execute_batch("CREATE TABLE book_rates(book_id INTEGER PRIMARY KEY REFERENCES books(id) ON DELETE CASCADE,value REAL NOT NULL CHECK(value BETWEEN 0.5 AND 3.0),updated INTEGER NOT NULL,device TEXT NOT NULL);")?;
    tx.pragma_update(None, "user_version", 7)?;
    tx.commit()?;
    Ok(())
}

impl Store {
    /// Keep received progress, speeds and their revisions in one transaction.
    pub fn merge_sync_exchange(
        &mut self,
        library: &Library,
        records: &[Record],
        rates: &[RateRecord],
    ) -> Result<(Library, Vec<Id>, Vec<Id>)> {
        let index = Index::new(library, &self.fingerprints()?);
        let tx = self.conn.transaction()?;
        let (library, changed) = apply_sync_records(&tx, &index, library, records)?;
        let rate_changes = apply_sync_rates(&tx, &index, rates)?;
        tx.commit()?;
        Ok((library, changed, rate_changes))
    }
    pub fn book_rate(&self, book: Id) -> Result<Option<f64>> {
        Ok(self
            .conn
            .query_row(
                "SELECT value FROM book_rates WHERE book_id=?1",
                [book],
                |r| r.get(0),
            )
            .optional()?)
    }
    pub fn save_book_rate(&mut self, book: Id, value: f64) -> Result<()> {
        anyhow::ensure!(
            value.is_finite()
                && (crate::player::MIN_RATE..=crate::player::MAX_RATE).contains(&value),
            "Invalid book speed"
        );
        self.conn.execute("INSERT INTO book_rates VALUES(?1,?2,?3,(SELECT id FROM sync_identity)) ON CONFLICT(book_id) DO UPDATE SET value=excluded.value,updated=MAX(excluded.updated,book_rates.updated+1),device=excluded.device WHERE book_rates.value!=excluded.value", params![book, value, crate::library::now_millis()])?;
        Ok(())
    }
    pub fn sync_rates(&self, library: &Library) -> Result<Vec<RateRecord>> {
        let index = Index::new(library, &self.fingerprints()?);
        let values = self
            .conn
            .prepare("SELECT book_id,value,updated,device FROM book_rates")?
            .query_map([], |r| {
                Ok((
                    r.get::<_, Id>(0)?,
                    r.get::<_, f64>(1)?,
                    r.get::<_, i64>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let rates: Vec<_> = values
            .into_iter()
            .filter_map(|(id, value, updated, device)| {
                index.books.get(&id).map(|book| RateRecord {
                    book: book.key.clone(),
                    value,
                    updated,
                    device,
                })
            })
            .collect();
        anyhow::ensure!(
            rates.len() <= crate::sync::MAX_RECORDS,
            "Too many book speeds"
        );
        Ok(rates)
    }
    pub fn merge_sync_rates(&mut self, library: &Library, rates: &[RateRecord]) -> Result<Vec<Id>> {
        let index = Index::new(library, &self.fingerprints()?);
        let tx = self.conn.transaction()?;
        let changed = apply_sync_rates(&tx, &index, rates)?;
        tx.commit()?;
        Ok(changed)
    }
    pub fn sync_device(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("SELECT id FROM sync_identity", [], |r| r.get(0))?)
    }
    pub fn fingerprints(&self) -> Result<Vec<Fingerprint>> {
        Ok(self
            .conn
            .prepare("SELECT file_id,signature,hash FROM sync_fingerprints")?
            .query_map([], |r| {
                Ok(Fingerprint {
                    file: r.get(0)?,
                    signature: r.get(1)?,
                    hash: r.get(2)?,
                })
            })?
            .collect::<Result<_, _>>()?)
    }
    pub fn save_fingerprints(
        &mut self,
        library: &Library,
        fingerprints: &[Fingerprint],
    ) -> Result<()> {
        let tx = self.conn.transaction()?;
        for f in fingerprints {
            if library.media(f.file).is_some_and(|m| f.matches(m)) {
                tx.execute("INSERT INTO sync_fingerprints VALUES(?1,?2,?3) ON CONFLICT(file_id) DO UPDATE SET signature=excluded.signature,hash=excluded.hash",
                    params![f.file, f.signature, f.hash])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn sync_records(&self, library: &Library) -> Result<Vec<Record>> {
        let index = Index::new(library, &self.fingerprints()?);
        let progress: std::collections::BTreeMap<_, _> =
            library.progress.iter().map(|p| (p.book_id, p)).collect();
        let changes: Vec<(Id, i64, String, bool)> = self
            .conn
            .prepare("SELECT book_id,updated,device,reset FROM sync_changes")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
            .collect::<Result<_, _>>()?;
        let mut records = Vec::new();
        for (id, updated, device, reset) in changes {
            let Some(book) = index.books.get(&id) else {
                continue;
            };
            let progress = progress.get(&id);
            let part = if reset {
                book.parts.first()
            } else {
                progress.and_then(|p| book.parts.iter().find(|(id, _, _)| *id == p.part_id))
            };
            let Some((_, part, occurrence)) = part else {
                continue;
            };
            records.push(Record {
                book: book.key.clone(),
                part: part.clone(),
                occurrence: *occurrence,
                position: if reset { 0 } else { progress.unwrap().position },
                completed: !reset && progress.unwrap().completed,
                reset,
                updated,
                device,
            });
        }
        anyhow::ensure!(
            records.len() <= crate::sync::MAX_RECORDS,
            "Too many books to synchronize"
        );
        Ok(records)
    }
    /// Apply progress and its original revision in one transaction. Receiving a
    /// remote value must never turn it into a newer local listening event.
    pub fn merge_sync(
        &mut self,
        library: &Library,
        records: &[Record],
    ) -> Result<(Library, Vec<Id>)> {
        let index = Index::new(library, &self.fingerprints()?);
        let tx = self.conn.transaction()?;
        let result = apply_sync_records(&tx, &index, library, records)?;
        tx.commit()?;
        Ok(result)
    }
}

fn apply_sync_rates(conn: &Connection, index: &Index, rates: &[RateRecord]) -> Result<Vec<Id>> {
    anyhow::ensure!(
        rates.len() <= crate::sync::MAX_RECORDS,
        "Too many book speeds"
    );
    for rate in rates {
        rate.validate()?;
    }
    let mut changed = Vec::new();
    let books = index.books_by_key();
    for rate in rates {
        for &id in books.get(rate.book.as_str()).into_iter().flatten() {
            if conn.execute("INSERT INTO book_rates VALUES(?1,?2,?3,?4) ON CONFLICT(book_id) DO UPDATE SET value=excluded.value,updated=excluded.updated,device=excluded.device WHERE (excluded.updated,excluded.device)>(book_rates.updated,book_rates.device)", params![id, rate.value, rate.updated, rate.device])? > 0 {
                changed.push(id);
            }
        }
    }
    Ok(changed)
}

fn apply_sync_records(
    tx: &Connection,
    index: &Index,
    library: &Library,
    records: &[Record],
) -> Result<(Library, Vec<Id>)> {
    anyhow::ensure!(
        records.len() <= crate::sync::MAX_RECORDS,
        "Too many sync records"
    );
    for record in records {
        record.validate()?;
    }
    let books = index.books_by_key();
    let parts: BTreeMap<_, _> = library.parts.iter().map(|p| (p.id, p)).collect();
    let media: BTreeMap<_, _> = library.media.iter().map(|m| (m.id, m)).collect();
    let mut library = library.clone();
    let mut changed = Vec::new();
    let mut progress_updates = BTreeMap::new();
    for record in records {
        for &id in books.get(record.book.as_str()).into_iter().flatten() {
            let book = &index.books[&id];
            let previous: Option<(i64, String)> = tx
                .query_row(
                    "SELECT updated,device FROM sync_changes WHERE book_id=?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            if previous
                .as_ref()
                .is_some_and(|(time, device)| (*time, device.as_str()) >= record.revision())
            {
                continue;
            }
            let Some((part, _, _)) = book.parts.iter().find(|(_, hash, occurrence)| {
                hash == &record.part && *occurrence == record.occurrence
            }) else {
                continue;
            };
            let duration = parts
                .get(part)
                .and_then(|p| media.get(&p.file_id))
                .and_then(|m| m.duration);
            // Metadata durations can differ slightly between platform decoders.
            if duration.is_some_and(|d| record.position > d.saturating_add(2_000)) {
                continue;
            }
            let active = library.session.current.as_ref().and_then(|t| match t {
                Target::Book(p) => parts.get(p).map(|p| p.book_id),
            }) == Some(id);
            if record.reset {
                tx.execute("DELETE FROM book_progress WHERE book_id=?1", [id])?;
                if active {
                    library.session.current = None;
                    library.session.position = 0;
                }
            } else {
                tx.execute("INSERT INTO book_progress VALUES(?1,?2,?3,?4,?5) ON CONFLICT(book_id) DO UPDATE SET part_id=excluded.part_id,position=excluded.position,completed=excluded.completed,updated=excluded.updated",
                        params![id, part, record.position as i64, record.completed, record.updated])?;
                if active {
                    library.session.current = Some(Target::Book(*part));
                    library.session.position = record.position;
                }
            }
            tx.execute("INSERT INTO sync_changes VALUES(?1,?2,?3,?4) ON CONFLICT(book_id) DO UPDATE SET updated=excluded.updated,device=excluded.device,reset=excluded.reset",
                    params![id, record.updated, record.device, record.reset])?;
            let progress = (!record.reset).then_some(Progress {
                book_id: id,
                part_id: *part,
                position: record.position,
                completed: record.completed,
                updated: record.updated,
            });
            // Keep the last accepted update per book, in its original arrival
            // order. Rebuilding once avoids scanning all progress per record.
            progress_updates.insert(id, (changed.len(), progress));
            changed.push(id);
        }
    }
    library
        .progress
        .retain(|p| !progress_updates.contains_key(&p.book_id));
    let mut updated: Vec<_> = progress_updates
        .into_values()
        .filter_map(|(order, progress)| progress.map(|p| (order, p)))
        .collect();
    updated.sort_by_key(|(order, _)| *order);
    library.progress.extend(updated.into_iter().map(|(_, p)| p));
    save_session(tx, &library.session)?;
    Ok((library, changed))
}
