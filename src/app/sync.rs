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
        match self
            .store()
            .and_then(|s| Ok((s.sync_records(&library)?, s.sync_rates(&library)?)))
        {
            Ok((records, rates)) => self.sync.as_ref().unwrap().publish(records, rates),
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
        files.sort_by_key(|file| {
            self.library
                .parts
                .iter()
                .filter(|p| p.file_id == file.id)
                .map(|part| {
                    if active == Some(part.book_id) {
                        0
                    } else if self
                        .library
                        .progress
                        .iter()
                        .any(|p| p.book_id == part.book_id)
                    {
                        1
                    } else {
                        2
                    }
                })
                .min()
                .unwrap_or(3)
        });
        self.sync_hash_control = ScanControl::default();
        for file in &files {
            self.sync_hash_attempted
                .insert((file.id, Fingerprint::signature(file)));
        }
        let cancelled = self.sync_hash_control.cancelled.clone();
        let tx = self.tx.clone();
        let worker = std::thread::Builder::new()
            .name("carlitos-fingerprints".into())
            .spawn(move || {
                let mut failed = false;
                for media in files {
                    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    if let Ok(fingerprint) = Fingerprint::read(&media, &cancelled) {
                        let _ = tx.send(Command::SyncFingerprints(vec![fingerprint], false));
                    } else { failed = true; }
                }
                if failed && !cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = tx.send(Command::Error(text("Не удалось подготовить часть книг к синхронизации. Проверьте доступ к файлам и нажмите «Синхронизировать».", "Some books could not be prepared for synchronization. Check file access and choose Sync.").into()));
                }
                let _ = tx.send(Command::SyncFingerprints(vec![], true));
            })?;
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
            // the previous position back as a fresh local update.
            if let Some(Target::Book(part)) = self.library.session.current {
                self.load(part, self.library.session.position, false)?;
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
