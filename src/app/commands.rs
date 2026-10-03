use super::audio::AudioCommand;
use super::{Command, Controller, Event, text};
use crate::library::{Target, parse_time};
#[cfg(target_os = "android")]
use crate::storage::Draft;
use anyhow::{Context, Result, bail};

impl Controller {
    pub(super) fn handle(&mut self, command: Command) -> Result<bool> {
        use Command::*;
        if self.quitting
            && !matches!(
                command,
                Playback(_)
                    | AudioFailed(..)
                    | Error(_)
                    | AudioStopped
                    | ScanDone(..)
                    | Relocated(..)
            )
        {
            return Ok(false);
        }
        if self.options.demo
            && matches!(
                command,
                Resume(_)
                    | Part(..)
                    | Toggle
                    | Playing(_)
                    | Stop
                    | Next(_)
                    | SeekDelta(_)
                    | SeekAbsolute(_)
                    | SetPosition(..)
                    | SeekFraction(_)
                    | Exact(_)
            )
        {
            self.notice(
                if cfg!(target_os = "android") {
                    text(
                        "Демонстрация интерфейса. Импорт и звук пока недоступны.",
                        "Interface demo. Import and audio are not available yet.",
                    )
                } else {
                    text(
                        "Демонстрация оформления. Добавьте свои книги в обычном режиме.",
                        "Design preview. Add your books in normal mode.",
                    )
                }
                .into(),
            );
            return Ok(false);
        }
        // A file returned by Android's picker imports immediately, without
        // passing through the folder preview's Import command.
        #[cfg(target_os = "android")]
        let imports_document = matches!(&command, OpenDocument(_));
        #[cfg(not(target_os = "android"))]
        let imports_document = false;
        if self.maintenance
            && (imports_document
                || matches!(
                    command,
                    Edit(..)
                        | MovePart(..)
                        | ResetProgress(_)
                        | RemoveBook(_)
                        | RemoveSource(_)
                        | Relocate(..)
                        | Rescan(_)
                        | Scan(..)
                        | Import
                ))
        {
            bail!(
                "{}",
                text(
                    "Дождитесь проверки нового расположения источника",
                    "Wait for source relocation to finish"
                )
            );
        }
        if self.scan_source.is_some()
            && (imports_document
                || matches!(
                    command,
                    RemoveBook(_) | RemoveSource(_) | ResetProgress(_) | Relocate(..) | Import
                ))
        {
            bail!(
                "{}",
                text(
                    "Дождитесь обновления источника",
                    "Wait for the source update to finish"
                )
            );
        }
        match command {
            #[cfg(all(target_os = "android", feature = "android-playback-tests"))]
            PlaybackTest(request, reply) => {
                let result = self.playback_test(request);
                let _ = reply.send(result);
            }
            #[cfg(target_os = "android")]
            Picked(kind, uri, name, owner) => {
                let _ = self.events.try_send(Event::Picked(kind, uri, name, owner));
            }
            #[cfg(target_os = "android")]
            OpenDocument(file) => {
                self.save()?;
                let uri = file.uri.clone();
                let identity = file.identity.clone();
                let library = self.store()?.import(vec![Draft {
                    root_uri: uri.clone(),
                    title: file.title.clone(),
                    author: file.artist.clone(),
                    files: vec![file],
                    include: true,
                }])?;
                self.reconcile(library);
                let part = self
                    .library
                    .parts
                    .iter()
                    .find(|part| {
                        self.library
                            .media(part.file_id)
                            .is_some_and(|file| file.identity == identity)
                    })
                    .context("Selected document has no part")?
                    .id;
                let position = self
                    .library
                    .progress
                    .iter()
                    .find(|p| p.part_id == part && !p.completed)
                    .map_or(0, |p| p.position);
                self.load(part, position, true)?;
                let _ = self.events.try_send(Event::Imported);
            }
            Resume(book) => self.resume(book)?,
            Part(id, position) => self.load(id, position, true)?,
            Toggle => self.play(!self.playback.playing)?,
            Playing(value) => self.play(value)?,
            Stop => {
                self.save()?;
                self.stop();
                self.save()?;
                self.emit_library();
            }
            Next(forward) => {
                if let Some(Target::Book(id)) = self.library.neighbour(forward) {
                    self.load(id, 0, self.playback.playing)?;
                }
            }
            SeekDelta(delta) => self.seek_delta(delta)?,
            SeekFraction(fraction) => {
                if fraction.is_finite()
                    && let Some(duration) = self.playback.duration
                {
                    self.seek_to((fraction.clamp(0., 1.) * duration as f64) as u64);
                }
            }
            SeekAbsolute(position) => {
                if self.playback.duration.is_none_or(|d| position <= d) {
                    self.seek_to(position);
                }
            }
            SetPosition(part, uri, position) => {
                // Recheck the track on the controller thread: a queued part
                // change can overtake the desktop thread's metadata snapshot.
                if self.library.session.current == Some(Target::Book(part))
                    && self
                        .library
                        .active_file()
                        .is_some_and(|file| file.uri == uri)
                {
                    return self.handle(SeekAbsolute(position));
                }
            }
            Exact(value) => {
                let position =
                    parse_time(&value, self.playback.duration.unwrap_or(0)).context(text(
                        "Введите время в пределах текущей части: мм:сс или чч:мм:сс",
                        "Enter a time within this part: mm:ss or hh:mm:ss",
                    ))?;
                self.seek_to(position);
            }
            Volume(volume) => self.set_volume(volume, self.library.session.muted)?,
            Rate(rate) => self.set_rate(rate)?,
            RateStep(forward) => self.step_rate(forward)?,
            SkipSilence(enabled) => self.set_skip_silence(enabled)?,
            Mute => self.set_volume(self.library.session.volume, !self.library.session.muted)?,
            Complete(book) => self.complete(book)?,
            ResetProgress(book) => self.reset_progress(book)?,
            PrepareCover(request, path) => self.prepare_cover(request, path)?,
            TagsRead(files) => self.tags_read(files)?,
            MovePart(id, forward) => self.move_part(id, forward)?,
            Edit(id, title, author, order, cover) => self.edit(id, title, author, order, cover)?,
            RemoveBook(id) => {
                self.save()?;
                let library = self.store()?.remove_book(id)?;
                self.reconcile(library);
                self.save()?;
            }
            RemoveSource(id) => {
                self.save()?;
                let library = self.store()?.remove_source(id)?;
                self.reconcile(library);
                self.save()?;
            }
            Relocate(id, path) => self.relocate(id, path)?,
            Relocated(id, result) => self.relocated(id, result)?,
            Rescan(id) => {
                let source = self
                    .library
                    .sources
                    .iter()
                    .find(|s| s.id == id)
                    .context("Source not found")?;
                let single = crate::source::document_identity(&source.uri).is_some()
                    && self.library.books.iter().any(|book| book.source_id == id);
                self.scan(crate::source::Location::uri(&source.uri)?, single)?;
                self.scan_source = Some(id);
                let _ = self.events.try_send(Event::SourceUpdating(true));
            }
            Scan(path, single) => self.scan(path, single)?,
            CancelScan => {
                self.scan_control.cancel();
                if self.scan_source.take().is_some() {
                    let _ = self.events.try_send(Event::SourceUpdating(false));
                }
                self.scan_generation += 1;
                self.drafts.clear();
                self.issues.clear();
                self.emit_drafts();
                let _ = self
                    .events
                    .try_send(Event::Scanning(false, text("Отменено", "Cancelled").into()));
            }
            ScanDone(generation, result) => self.scan_done(generation, result)?,
            DraftEdit(index, title, author, included) => {
                if let Some(draft) = self.drafts.get_mut(index) {
                    draft.title = title;
                    draft.author = author;
                    draft.include = included;
                    self.emit_drafts();
                }
            }
            DraftFile(draft, uri, action) => {
                if let Some(draft) = self.drafts.get_mut(draft)
                    && let Some(index) = draft.files.iter().position(|file| file.uri == uri)
                {
                    match action.as_str() {
                        "up" if index > 0 => draft.files.swap(index, index - 1),
                        "down" if index + 1 < draft.files.len() => {
                            draft.files.swap(index, index + 1)
                        }
                        "remove" => {
                            draft.files.remove(index);
                        }
                        _ => {}
                    }
                    self.emit_drafts();
                }
            }
            Import => self.import()?,
            Settings(mut settings) => {
                // Rate is confirmed by the audio worker, independently of appearance changes.
                settings.playback_rate = self.options.settings.playback_rate;
                settings.skip_silence = self.options.settings.skip_silence;
                settings.validate()?;
                let saved = if self.options.settings_writable && !self.options.demo {
                    settings.save(&self.options.settings_path)
                } else {
                    Ok(())
                };
                // The view already applied these preferences. Keep the live
                // controller consistent even when persisting them fails.
                // The UI applies the language synchronously. An older queued
                // Settings command must not change its locale while it builds
                // labels for a newer selection; Event::Settings is applied on
                // the UI thread along with the rest of the preferences.
                self.options.settings = settings.clone();
                let _ = self.events.try_send(Event::Settings(settings));
                self.emit_library();
                saved?;
            }
            Playback(snapshot) => return self.receive_playback(snapshot),
            AudioFailed(token, message) => return self.receive_audio_failure(token, message),
            Error(message) => self.notice(message),
            AudioStopped => {
                self.playback.playing = false;
                self.playback.seekable = false;
                self.playback.phase = crate::player::Phase::Error;
                let _ = self.events.try_send(Event::Playback(self.playback.clone()));
                if self.store.as_ref().is_some_and(|store| store.is_ok()) {
                    self.save()?;
                }
                if self.quitting {
                    self.finish()?;
                    return Ok(true);
                }
            }
            Hidden(hidden) => {
                self.pending_save = true;
                self.audio.send(AudioCommand::Snapshot);
                let _ = self.events.try_send(Event::Hidden(hidden));
            }
            Show => {
                let _ = self.events.try_send(Event::Show);
            }
            Quit => {
                self.scan_control.cancel();
                self.maintenance_control.cancel();
                self.tag_control.cancel();
                self.quitting = true;
                self.pending_save = true;
                if self.playback.phase == crate::player::Phase::Empty
                    || self.options.demo
                    || self.audio.is_closed()
                {
                    self.finish()?;
                    return Ok(true);
                }
                self.audio.send(AudioCommand::Snapshot);
            }
        }
        Ok(false)
    }
}
