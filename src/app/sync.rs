use super::{Command, Controller, Event, text};
use crate::{
    import::ScanControl,
    library::Target,
    sync::{Fingerprint, Network, Status},
};
use anyhow::Result;

impl Controller {
    pub(super) fn configure_sync(&mut self) -> Result<()> {
        let enabled = self.options.settings.local_sync && !self.options.demo;
        if enabled && self.sync.is_none() {
            self.sync_hash_attempted.clear();
            let device = self.store()?.sync_device()?;
            let path = self.options.database.with_extension("sync.json");
            #[cfg(target_os = "android")]
            crate::android::bridge::dispatch(
                serde_json::json!({"op":"sync-network", "enabled":true}),
            )?;
            match Network::start(path, device, self.tx.clone()) {
                Ok(network) => self.sync = Some(network),
                Err(error) => {
                    #[cfg(target_os = "android")]
                    let _ = crate::android::bridge::dispatch(
                        serde_json::json!({"op":"sync-network", "enabled":false}),
                    );
                    return Err(error);
                }
            }
            self.refresh_sync()?;
        } else if !enabled {
            self.sync.take();
            self.sync_hash_control.cancel();
            let _ = self.events.try_send(Event::SyncIndexing(false));
            let _ = self.events.try_send(Event::SyncReady(vec![]));
            #[cfg(target_os = "android")]
            crate::android::bridge::dispatch(
                serde_json::json!({"op":"sync-network", "enabled":false}),
            )?;
            let _ = self.events.try_send(Event::SyncStatus(Status::default()));
        }
        Ok(())
    }
    pub(super) fn publish_sync(&mut self) {
        if self.sync.is_none() {
            return;
        }
        let library = self.library.clone();
        match self.store().and_then(|s| {
            Ok((
                s.sync_records(&library)?,
                s.sync_rates(&library)?,
                crate::sync::Index::new(&library, &s.fingerprints()?)
                    .books
                    .into_keys()
                    .collect(),
            ))
        }) {
            Ok((records, rates, ready)) => {
                let _ = self.events.try_send(Event::SyncReady(ready));
                self.sync.as_ref().unwrap().publish(records, rates);
            }
            Err(error) => self.notice(format!(
                "{}: {error:#}",
                text("Синхронизация", "Synchronization")
            )),
        }
    }
    pub(super) fn refresh_sync(&mut self) -> Result<()> {
        if self.sync.is_none() {
            return Ok(());
        }
        self.publish_sync();
        if self.sync_hashing {
            return Ok(());
        }
        let fingerprints: std::collections::HashMap<_, _> = self
            .store()?
            .fingerprints()?
            .into_iter()
            .map(|f| (f.file, f))
            .collect();
        let mut files: Vec<_> = self
            .library
            .media
            .iter()
            // Removed books can leave cached media behind. Only files belonging
            // to a current book contribute to its synchronization identity.
            .filter(|m| self.library.parts.iter().any(|p| p.file_id == m.id))
            .filter(|m| !fingerprints.get(&m.id).is_some_and(|f| f.matches(m)))
            .filter(|m| {
                !self
                    .sync_hash_attempted
                    .contains(&(m.id, Fingerprint::signature(m)))
            })
            .cloned()
            .collect();
        if files.is_empty() {
            return Ok(());
        }
        let active = self.active_book();
        let started: std::collections::HashSet<_> =
            self.library.progress.iter().map(|p| p.book_id).collect();
        let mut priorities = std::collections::HashMap::<_, u8>::new();
        for part in &self.library.parts {
            let priority = if active == Some(part.book_id) {
                0
            } else if started.contains(&part.book_id) {
                1
            } else {
                2
            };
            priorities
                .entry(part.file_id)
                .and_modify(|old| *old = (*old).min(priority))
                .or_insert(priority);
        }
        files.sort_by_key(|file| priorities.get(&file.id).copied().unwrap_or(3));
        self.sync_hash_control = ScanControl::default();
        let attempted: Vec<_> = files
            .iter()
            .map(|file| (file.id, Fingerprint::signature(file)))
            .collect();
        let cancelled = self.sync_hash_control.cancelled.clone();
        let tx = self.tx.clone();
        self.reap_workers();
        let worker = std::thread::Builder::new()
            .name("carlitos-fingerprints".into())
            .spawn(move || {
                let mut failures = Vec::new();
                for media in files {
                    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    match Fingerprint::read(&media, &cancelled) {
                        Ok(fingerprint) => {
                            let _ = tx.send(Command::SyncFingerprints(vec![fingerprint], false));
                        }
                        Err(error) => failures.push(format!("{}: {error:#}", media.relative)),
                    }
                }
                if !failures.is_empty() && !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(Command::Error(format!(
                        "{} {}. {} {}",
                        text(
                            "Не удалось подготовить файлы к синхронизации:",
                            "Could not prepare files for synchronization:"
                        ),
                        failures.len(),
                        failures.join("; "),
                        text(
                            "Проверьте доступ к этим файлам и включите синхронизацию снова.",
                            "Check access to these files and start synchronization again."
                        )
                    )));
                }
                let _ = tx.send(Command::SyncFingerprints(vec![], true));
            })?;
        // A failed thread spawn did not attempt any file and must not suppress
        // subsequent background retries for the same signatures.
        self.sync_hash_attempted.extend(attempted);
        self.sync_hashing = true;
        let _ = self.events.try_send(Event::SyncIndexing(true));
        self.scans.push(worker);
        Ok(())
    }
    pub(super) fn receive_sync(
        &mut self,
        records: Vec<crate::sync::Record>,
        rates: Vec<crate::sync::RateRecord>,
    ) -> Result<()> {
        if (records.is_empty() && rates.is_empty())
            || self.sync.is_none()
            || self.maintenance
            || self.scan_source.is_some()
            || (self.restore_progress.is_none()
                && matches!(
                    self.playback.phase,
                    crate::player::Phase::Loading | crate::player::Phase::Seeking
                ))
        {
            return Ok(());
        }
        // Flush the latest confirmed local snapshot before comparing revisions.
        self.save()?;
        let active = self.active_book();
        let library = self.library.clone();
        let (library, changed, rate_changes) = self
            .store()?
            .merge_sync_exchange(&library, &records, &rates)?;
        if changed.is_empty() && rate_changes.is_empty() {
            return Ok(());
        }
        let active_changed = active.is_some_and(|id| changed.contains(&id));
        if active.is_some_and(|id| rate_changes.contains(&id)) {
            self.pending_rate = None;
        }
        self.library = library;
        if active_changed {
            // Invalidate already queued decoder events before they can write
            // the previous position back as a fresh local update. A load can
            // fail before advancing its token (closed audio, failed DB read).
            if let Some(Target::Book(part)) = self.library.session.current {
                let position = self.library.session.position;
                let pending_rate = self.pending_rate;
                self.stop();
                self.pending_rate = pending_rate;
                if let Err(error) = self.load(part, position, false) {
                    // The remote progress is already committed. Publish it
                    // even when its paused player could not be restored.
                    self.emit_library();
                    self.publish_sync();
                    return Err(error);
                }
            } else {
                self.stop();
            }
            self.notice(
                text(
                    "Позиция обновлена с другого устройства",
                    "Position updated from another device",
                )
                .into(),
            );
        }
        if let Some(book) = active.filter(|id| rate_changes.contains(id))
            && !active_changed
            && let Some(rate) = self.store()?.book_rate(book)?
        {
            self.apply_rate(rate)?;
        }
        self.emit_library();
        self.publish_sync();
        Ok(())
    }
}
