use super::{Command, Controller, Event, text};
use crate::source::Location;
use crate::{
    import::{self, ImportMode, ScanControl, ScanResult},
    library::*,
    storage::{Draft, Store},
};
use anyhow::{Result, bail};

impl Controller {
    pub(super) fn scan(&mut self, path: Location, single: bool) -> Result<()> {
        if let Location::File(path) = &path
            && (!path.is_absolute() || !path.is_dir())
        {
            bail!(
                "{}",
                text(
                    "Укажите полный путь к существующей папке",
                    "Enter an absolute path to an existing folder"
                )
            );
        }
        self.scan_control.cancel();
        if self.scan_source.take().is_some() {
            let _ = self.events.try_send(Event::SourceUpdating(false));
        }
        self.scan_generation += 1;
        let generation = self.scan_generation;
        self.scan_control = ScanControl::default();
        self.drafts.clear();
        self.issues.clear();
        self.emit_drafts();
        let _ = self.events.try_send(Event::Scanning(
            true,
            text("Сканирование…", "Scanning…").into(),
        ));
        let control = self.scan_control.clone();
        let tx = self.tx.clone();
        self.scans.retain(|thread| !thread.is_finished());
        let worker = std::thread::Builder::new()
            .name("carlitos-import".into())
            .spawn(move || {
                let result = import::scan_location(
                    path,
                    if single {
                        ImportMode::Book
                    } else {
                        ImportMode::Books
                    },
                    control,
                );
                let _ = tx.send(Command::ScanDone(generation, result));
            });
        match worker {
            Ok(worker) => self.scans.push(worker),
            Err(error) => {
                let _ = self.events.try_send(Event::Scanning(false, String::new()));
                return Err(error.into());
            }
        }
        Ok(())
    }
    pub(super) fn relocate(&mut self, id: Id, path: Location) -> Result<()> {
        self.save()?;
        self.store()?;
        self.handle(Command::CancelScan)?;
        self.maintenance = true;
        self.maintenance_control = ScanControl::default();
        let _ = self.events.try_send(Event::SourceUpdating(true));
        let database = self.options.database.clone();
        let tx = self.tx.clone();
        let control = self.maintenance_control.clone();
        let worker = std::thread::Builder::new()
            .name("carlitos-relocate".into())
            .spawn(move || {
                let result = Store::open(&database)
                    .and_then(|mut store| store.relocate_location(id, &path, &control))
                    .map(|_| ());
                let _ = tx.send(Command::Relocated(id, result));
            });
        match worker {
            Ok(worker) => self.scans.push(worker),
            Err(error) => {
                self.maintenance = false;
                let _ = self.events.try_send(Event::SourceUpdating(false));
                return Err(error.into());
            }
        }
        Ok(())
    }
    pub(super) fn relocated(&mut self, id: Id, result: Result<()>) -> Result<()> {
        self.maintenance = false;
        let _ = self.events.try_send(Event::SourceUpdating(false));
        if result.is_err()
            && (self.quitting
                || self
                    .maintenance_control
                    .cancelled
                    .load(std::sync::atomic::Ordering::Relaxed))
        {
            return Ok(());
        }
        result?;
        let mut library = self.store()?.load()?;
        // Relocation has already committed. Refresh its paths even if
        // the next checkpoint fails, retaining progress made while
        // the background worker was validating the new location.
        library.session = self.library.session.clone();
        library.progress = self.library.progress.clone();
        self.reconcile(library);
        self.save()?;
        if !self.quitting {
            self.handle(Command::Rescan(id))?;
        }
        Ok(())
    }
    pub(super) fn scan_done(&mut self, generation: u64, result: Result<ScanResult>) -> Result<()> {
        if generation == self.scan_generation {
            let _ = self.events.try_send(Event::Scanning(false, String::new()));
            let source = self.scan_source.take();
            if source.is_some() {
                let _ = self.events.try_send(Event::SourceUpdating(false));
            }
            if self.quitting
                || self
                    .scan_control
                    .cancelled
                    .load(std::sync::atomic::Ordering::Relaxed)
            {
                return Ok(());
            }
            if let Some(source) = source {
                let result = result?;
                let drafts = preserve_source_books(&self.library, result.drafts)?;
                self.save()?;
                let library = self.store()?.import(drafts)?;
                self.reconcile(library);
                let mut message = text("Источник обновлён", "Source updated").to_string();
                if !result.issues.is_empty() {
                    message.push('\n');
                    message.push_str(&result.issues.join("\n"));
                }
                let _ = self.events.try_send(Event::SourceUpdated(source, message));
                return Ok(());
            }
            let result = result?;
            self.drafts = result.drafts;
            self.issues = result.issues;
            self.emit_drafts();
        }
        Ok(())
    }
    pub(super) fn import(&mut self) -> Result<()> {
        let drafts: Vec<_> = self
            .drafts
            .iter()
            .filter(|d| d.include && !d.files.is_empty())
            .cloned()
            .collect();
        if drafts.is_empty() {
            bail!(
                "{}",
                text("Выберите хотя бы одну книгу", "Select at least one book")
            );
        }
        if drafts.iter().any(|d| d.title.trim().is_empty()) {
            bail!("{}", text("Укажите названия книг", "Enter book titles"));
        }
        self.save()?;
        let library = self.store()?.import(drafts)?;
        self.reconcile(library);
        self.drafts.clear();
        self.issues.clear();
        self.emit_drafts();
        self.notice(
            text(
                "Книги добавлены в библиотеку",
                "Books added to your library",
            )
            .into(),
        );
        let _ = self.events.try_send(Event::Imported);
        Ok(())
    }
}

/// Refresh known audiobooks as they were imported, including manually combined
/// folders. Collection roots with separate child sources still discover new books.
fn preserve_source_books(library: &Library, drafts: Vec<Draft>) -> Result<Vec<Draft>> {
    use std::collections::{BTreeMap, HashMap, HashSet};
    let paths: HashMap<_, _> = library
        .sources
        .iter()
        .filter_map(|s| local_path(&s.uri).map(|p| (s.id, p)))
        .collect();
    let roots: HashSet<_> = library
        .books
        .iter()
        .filter_map(|book| paths.get(&book.source_id))
        .map(|path| path.as_path())
        .collect();
    let parents: HashSet<_> = roots
        .iter()
        .flat_map(|path| path.ancestors().skip(1))
        .filter(|path| roots.contains(path))
        .collect();
    let books: HashSet<_> = roots.difference(&parents).copied().collect();
    let mut grouped = BTreeMap::<String, Draft>::new();
    for mut draft in drafts {
        if let Some(path) = local_path(&draft.root_uri)
            && let Some(root) = path.ancestors().find(|p| books.contains(p))
        {
            draft.root_uri = file_uri(root)?;
            for file in &mut draft.files {
                if let Some(path) = local_path(&file.uri) {
                    file.relative = path.strip_prefix(root)?.to_string_lossy().into_owned();
                }
            }
        }
        if let Some(existing) = grouped.get_mut(&draft.root_uri) {
            existing.files.extend(draft.files);
        } else {
            grouped.insert(draft.root_uri.clone(), draft);
        }
    }
    Ok(grouped
        .into_values()
        .map(|mut draft| {
            order_parts(&mut draft.files);
            draft
        })
        .collect())
}
