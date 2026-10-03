use super::{Command, Controller, Event, text};
use crate::{import, library::*};
use anyhow::{Context, Result, bail};

impl Controller {
    pub(super) fn refresh_missing_tags(&mut self) {
        let files: Vec<_> = self
            .library
            .media
            .iter()
            .filter(|m| !m.sort_tags_read)
            .cloned()
            .collect();
        if files.is_empty() {
            return;
        }
        let tx = self.tx.clone();
        let cancelled = self.tag_control.cancelled.clone();
        if let Ok(worker) = std::thread::Builder::new()
            .name("carlitos-tags".into())
            .spawn(move || {
                let Ok(discoverer) = import::discoverer() else {
                    return;
                };
                let mut batch = Vec::new();
                for file in files {
                    if cancelled.load(std::sync::atomic::Ordering::Relaxed) {
                        return;
                    }
                    if let Ok(tags) = import::read_sort_tags(&file.uri, &discoverer) {
                        batch.push((file, tags));
                        if batch.len() == 16 {
                            let _ = tx.send(Command::TagsRead(std::mem::take(&mut batch)));
                        }
                    }
                }
                if !batch.is_empty() {
                    let _ = tx.send(Command::TagsRead(batch));
                }
            })
        {
            self.scans.push(worker);
        }
    }
    pub(super) fn complete(&mut self, book: Id) -> Result<()> {
        let active = self.active_book() == Some(book);
        let last = self
            .library
            .book_parts(book)
            .last()
            .copied()
            .cloned()
            .context("Book has no parts")?;
        let position = self
            .library
            .media(last.file_id)
            .and_then(|m| m.duration)
            .unwrap_or(0);
        let mut progress = self.library.progress.clone();
        progress.retain(|p| p.book_id != book);
        progress.push(Progress {
            book_id: book,
            part_id: last.id,
            position,
            completed: true,
            updated: now(),
        });
        let mut session = self.library.session.clone();
        if active {
            session.current = None;
            session.position = 0;
        }
        // Commit before stopping audio or publishing the new progress. A
        // rejected write must leave the current listening session intact.
        if !self.options.demo {
            self.store()?.save(&session, &progress)?;
            self.checkpoint = std::time::Instant::now();
        }
        if active {
            self.stop();
            self.pending_save = false;
        }
        self.library.progress = progress;
        self.emit_library();
        Ok(())
    }
    pub(super) fn reset_progress(&mut self, book: Id) -> Result<()> {
        if !self.library.progress.iter().any(|p| p.book_id == book) {
            return Ok(());
        }
        self.save()?;
        if !self.options.demo {
            self.store()?.reset_book_progress(book)?;
        }
        // Commit first; a failed reset must leave the player and its
        // progress intact. Stop invalidates queued playback snapshots.
        if self.active_book() == Some(book) {
            self.stop();
            self.pending_save = false;
        }
        self.library.progress.retain(|p| p.book_id != book);
        self.emit_library();
        Ok(())
    }
    pub(super) fn prepare_cover(
        &mut self,
        request: u64,
        path: crate::source::Location,
    ) -> Result<()> {
        let directory = if self.options.demo {
            std::env::temp_dir().join(format!("carlitos-demo-covers-{}", std::process::id()))
        } else {
            self.options
                .database
                .parent()
                .context("Library directory missing")?
                .join("covers")
        };
        let events = self.events.clone();
        let worker = std::thread::Builder::new()
            .name("carlitos-custom-cover".into())
            .spawn(move || {
                let result = import::custom_cover_location(&path, &directory);
                let _ = events.try_send(Event::CoverPrepared(request, result));
            });
        match worker {
            Ok(worker) => self.scans.push(worker),
            Err(error) => {
                let _ = self
                    .events
                    .try_send(Event::CoverPrepared(request, Err(error.into())));
            }
        }
        Ok(())
    }
    pub(super) fn tags_read(&mut self, files: Vec<(Media, BookTags)>) -> Result<()> {
        let mut changed = false;
        for (original, tags) in files {
            if self.store()?.update_sort_tags(&original, &tags)?
                && let Some(file) = self
                    .library
                    .media
                    .iter_mut()
                    .find(|m| m.id == original.id && m.uri == original.uri)
            {
                file.year = tags.year;
                file.genre = tags.genre;
                file.sort_tags_read = true;
                changed = true;
            }
        }
        if changed {
            self.emit_library();
        }
        Ok(())
    }
    pub(super) fn move_part(&mut self, id: Id, forward: bool) -> Result<()> {
        let part = self.library.part(id).context("Part not found")?;
        let book = self
            .library
            .books
            .iter()
            .find(|b| b.id == part.book_id)
            .context("Book not found")?
            .clone();
        let mut order: Vec<_> = self
            .library
            .book_parts(book.id)
            .iter()
            .map(|p| p.id)
            .collect();
        let index = order
            .iter()
            .position(|p| *p == id)
            .context("Part not found")?;
        let next = if forward {
            index.checked_add(1).filter(|i| *i < order.len())
        } else {
            index.checked_sub(1)
        };
        let Some(next) = next else {
            return Ok(());
        };
        order.swap(index, next);
        self.save()?;
        let library =
            self.store()?
                .edit_book(book.id, book.title, book.author, order, book.cover)?;
        self.reconcile(library);
        Ok(())
    }
    pub(super) fn edit(
        &mut self,
        id: Id,
        title: String,
        author: String,
        order: Vec<Id>,
        cover: Option<String>,
    ) -> Result<()> {
        if title.trim().is_empty() {
            bail!(
                "{}",
                text("Название не может быть пустым", "Title cannot be empty")
            );
        }
        self.save()?;
        let order = if order.is_empty() {
            self.library.book_parts(id).iter().map(|p| p.id).collect()
        } else {
            order
        };
        let library =
            self.store()?
                .edit_book(id, title.trim().into(), author.trim().into(), order, cover)?;
        self.reconcile(library);
        let _ = self.events.try_send(Event::BookEdited(id));
        Ok(())
    }
}
