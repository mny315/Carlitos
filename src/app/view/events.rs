use super::View;
use crate::{
    State,
    app::{Event, text},
    library::Target,
};
use slint::ComponentHandle;

impl View {
    pub fn event(&mut self, event: Event) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<State>();
        match event {
            Event::SyncIndexing(active) => state.set_sync_indexing(active),
            Event::SyncStatus(status) => {
                state.set_sync_busy(status.busy);
                state.set_sync_error(status.error);
                state.set_sync_code(status.code.into());
                if state.get_sync_invitation() != status.invitation {
                    state.set_sync_qr(if status.invitation.is_empty() {
                        slint::Image::default()
                    } else {
                        crate::sync::qr_image(&status.invitation).unwrap_or_default()
                    });
                    state.set_sync_invitation(status.invitation.into());
                }
                state.set_sync_name(status.name.into());
                state.set_sync_message(status.message.into());
                let peers: Vec<_> = status
                    .peers
                    .into_iter()
                    .map(|p| crate::SyncPeer {
                        key: p.id.into(),
                        name: p.name.into(),
                        online: p.online,
                    })
                    .collect();
                state.set_sync_peers(slint::ModelRc::new(slint::VecModel::from(peers)));
            }
            #[cfg(target_os = "android")]
            Event::AndroidRefresh => {}
            #[cfg(target_os = "android")]
            Event::Picked(kind, uri, name, owner) => match kind.as_str() {
                "folder" => {
                    state.set_import_path(uri.into());
                    state.set_import_folder_name(name.into());
                }
                "source"
                    if state.get_overlay() == 5
                        && state.get_picker_owner() == owner
                        && !state.get_source_busy() =>
                {
                    state.set_relocate_path(uri.into());
                    state.set_source_error("".into());
                }
                "cover" if state.get_overlay() == 2 && state.get_picker_owner() == owner => {
                    self.action("preview-cover", &uri);
                }
                _ => {}
            },
            Event::Library(library) => {
                let part_changed = self.library.session.current != library.session.current;
                // A rescan can repair a cached image at the same path.
                self.covers.clear_failures();
                self.library = *library;
                self.reindex();
                state.set_loading(false);
                state.set_book_count(self.library.books.len() as i32);
                state.set_volume(self.library.session.volume as f32);
                state.set_muted(self.library.session.muted);
                self.refresh_books();
                self.refresh_selected();
                self.refresh_current();
                if part_changed {
                    // Seed the new book's time before its first audio snapshot.
                    // Loading must block seeks without flashing disabled styling
                    // or briefly showing the old book's duration.
                    state.set_playback_loading(state.get_active());
                    state.set_seekable(false);
                    self.refresh_position(
                        self.library.session.position,
                        self.library.active_file().and_then(|file| file.duration),
                    );
                }
                self.refresh_sources();
            }
            Event::Playback(p) => {
                self.playback = p;
                if self.playback.phase == crate::player::Phase::Ready {
                    let duration_changed = if let Some(duration) = self.playback.duration
                        && let Some(Target::Book(id)) = self.library.session.current
                        && let Some(part) = self.by_part.get(&id)
                        && let Some(file) = self.by_media.get(&self.library.parts[*part].file_id)
                        && self.library.media[*file].duration != Some(duration)
                    {
                        self.library.media[*file].duration = Some(duration);
                        true
                    } else {
                        false
                    };
                    let active_book = self
                        .library
                        .session
                        .current
                        .as_ref()
                        .and_then(|Target::Book(id)| self.library.part(*id))
                        .map(|p| p.book_id);
                    let previous_status = self
                        .library
                        .progress
                        .iter()
                        .find(|p| Some(p.book_id) == active_book)
                        .map(|p| p.completed);
                    let completed = (self.playback.ended && self.library.neighbour(true).is_none())
                        || (!self.playback.playing
                            && self
                                .playback
                                .duration
                                .is_some_and(|d| self.playback.position >= d)
                            && self
                                .library
                                .session
                                .current
                                .as_ref()
                                .and_then(|Target::Book(id)| self.library.part(*id))
                                .is_some_and(|part| {
                                    self.library
                                        .progress
                                        .iter()
                                        .any(|p| p.book_id == part.book_id && p.completed)
                                }));
                    self.library
                        .update_progress(self.playback.position, completed);
                    if duration_changed {
                        self.refresh_selected();
                    }
                    if (self.filter != 0 || self.settings.library_sort == "listening")
                        && active_book.is_some()
                        && previous_status != Some(completed)
                    {
                        self.refresh_books();
                    }
                }
                if !self.hidden {
                    self.refresh_playback();
                }
            }
            Event::Volume(volume, muted) => {
                self.library.session.volume = volume;
                self.library.session.muted = muted;
                state.set_volume(volume as f32);
                state.set_muted(muted);
            }
            Event::Drafts(drafts, issues) => {
                self.drafts = drafts;
                if !usize::try_from(state.get_selected_draft())
                    .is_ok_and(|index| index < self.drafts.len())
                {
                    state.set_selected_draft(-1);
                }
                self.refresh_drafts();
                state.set_import_issues(issues.join("\n").into());
                self.refresh_draft();
            }
            Event::Scanning(scanning, status) => {
                state.set_scanning(scanning);
                state.set_scan_status(status.into());
            }
            Event::Settings(settings) => {
                self.settings = settings;
                self.apply_settings();
            }
            Event::Notice(message) => {
                self.notice_timer.stop();
                if state.get_overlay() == 2 {
                    state.set_edit_cover_error(message.clone().into());
                }
                if state.get_overlay() == 5 {
                    state.set_source_error(message.into());
                } else {
                    state.set_notice(message.into());
                }
            }
            Event::SourceUpdating(busy) => {
                state.set_source_busy(busy);
            }
            Event::SourceUpdated(id, message) => {
                if state.get_overlay() == 5 && state.get_relocate_key() == id.to_string() {
                    state.set_overlay(0);
                }
                state.set_source_error("".into());
                state.set_notice(message.into());
                self.expire_notice();
            }
            Event::Imported => {
                state.set_page(0);
                state.set_selected_draft(-1);
                self.expire_notice();
            }
            Event::BookEdited(id) => {
                if state.get_overlay() == 2 && state.get_edit_key() == id.to_string() {
                    state.set_overlay(0);
                }
            }
            Event::CoverPrepared(request, result) => {
                if request == self.cover_request && state.get_overlay() == 2 {
                    state.set_cover_loading(false);
                    match result {
                        Ok(path) => {
                            self.covers.retry(&path);
                            state.set_edit_cover_path(path.clone().into());
                            state.set_edit_cover(self.covers.image(Some(&path)));
                        }
                        Err(error) => state.set_edit_cover_error(
                            format!(
                                "{}: {error:#}",
                                text("Не удалось открыть изображение", "Could not open image")
                            )
                            .into(),
                        ),
                    }
                }
            }
            Event::Show => {
                self.hidden = false;
                let _ = w.show();
                self.refresh_playback();
            }
            Event::Hidden(hidden) => {
                self.hidden = hidden;
                if hidden {
                    #[cfg(not(target_os = "android"))]
                    let _ = w.hide();
                } else {
                    #[cfg(not(target_os = "android"))]
                    let _ = w.show();
                    self.refresh_playback();
                }
            }
            Event::Quit => {
                let _ = slint::quit_event_loop();
            }
        }
    }
}
