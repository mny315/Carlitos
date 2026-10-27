use super::View;
use crate::app::Command;
use crate::library::{Id, format_time, parse_time};
use crate::source::Location;
use crate::{AppWindow, State};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

impl View {
    pub(super) fn bind(view: &Rc<RefCell<Self>>, window: &AppWindow) {
        let state = window.global::<State>();
        state.set_ui_visible(!cfg!(target_os = "android"));
        state.set_backdrop_glow(crate::visualizer::glow());
        state.on_backdrop_active(crate::visualizer::set_enabled);
        state.on_voice_bands(|| {
            #[cfg(all(target_os = "android", feature = "android-ui-tests"))]
            crate::android::ui_tests::record_backdrop_sample();
            crate::visualizer::bands()
        });
        state.on_display_path(|uri| crate::source::display_path(uri.as_str()).into());
        let weak = Rc::downgrade(view);
        state.on_search(move |q, f| {
            if let Some(v) = weak.upgrade() {
                let mut v = v.borrow_mut();
                if v.query != q.to_lowercase() || v.filter != f {
                    v.action("clear-book-selection", "");
                }
                v.query = q.to_lowercase();
                v.filter = f;
                let tab = match f {
                    0 => Some("all"),
                    1 => Some("started"),
                    _ => None,
                };
                if let Some(tab) = tab
                    && v.settings.library_tab != tab
                {
                    v.settings.library_tab = tab.into();
                    v.send(Command::Settings(v.settings.clone()));
                }
                v.refresh_books();
            }
        });
        let weak = Rc::downgrade(view);
        state.on_action(move |action, arg| {
            if let Some(v) = weak.upgrade() {
                v.borrow_mut().action(action.as_str(), arg.as_str());
            }
        });
        let tx = view.borrow().tx.clone();
        let weak = window.as_weak();
        state.on_seek(move |value| {
            if let Some(w) = weak.upgrade() {
                let state = w.global::<State>();
                state.set_position_text(
                    format_time((value.clamp(0., 1.) * state.get_duration_ms()) as u64).into(),
                );
            }
            let _ = tx.send(Command::SeekFraction(value as f64));
        });
        let tx = view.borrow().tx.clone();
        state.on_set_volume(move |value| {
            let _ = tx.send(Command::Volume(value as f64));
        });
        let tx = view.borrow().tx.clone();
        state.on_set_rate(move |value| {
            let _ = tx.send(Command::Rate((value as f64 * 100.).round() / 100.));
        });
        let weak = Rc::downgrade(view);
        state.on_settings_changed(move || {
            if let Some(v) = weak.upgrade() {
                v.borrow_mut().save_settings();
            }
        });
        let weak = Rc::downgrade(view);
        state.on_display_settings_preview(move || {
            if let Some(view) = weak.upgrade() {
                view.borrow_mut().preview_display_settings();
            }
        });
        let tx = view.borrow().tx.clone();
        let weak = window.as_weak();
        state.on_draft_changed(move |index, title, author, included| {
            if index >= 0
                && let Some(window) = weak.upgrade()
                && let Ok(generation) = window.global::<State>().get_draft_generation().parse()
            {
                let _ = tx.send(Command::DraftEdit(
                    generation,
                    index as usize,
                    title.into(),
                    author.into(),
                    included,
                ));
            }
        });
        let weak = window.as_weak();
        let weak_view = Rc::downgrade(view);
        state.on_draft_file_action(move |index, action| {
            if let Some(w) = weak.upgrade()
                && let Some(view) = weak_view.upgrade()
            {
                let draft = w.global::<State>().get_selected_draft();
                let view = view.borrow();
                if draft >= 0
                    && index >= 0
                    && let Some(file) = view
                        .drafts
                        .get(draft as usize)
                        .and_then(|draft| draft.files.get(index as usize))
                {
                    view.send(Command::DraftFile(
                        w.global::<State>()
                            .get_draft_generation()
                            .parse()
                            .unwrap_or(0),
                        draft as usize,
                        file.uri.clone(),
                        action.into(),
                    ));
                }
            }
        });
    }
    pub(super) fn action(&mut self, action: &str, arg: &str) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<State>();
        let id = arg.parse::<Id>().ok();
        let command = match action {
            "skip-silence" => Some(Command::SkipSilence(state.get_skip_silence())),
            "open-book" => {
                if let Some(id) = id.filter(|id| self.library.books.iter().any(|b| b.id == *id)) {
                    if state.get_book_selection() {
                        self.book_selection_action("clear-book-selection", "");
                    }
                    self.selected = Some(id);
                    state.set_page(1);
                    // A tapped library row is about to become hidden. Keep
                    // Back/keyboard events on the visible page, not that row.
                    w.invoke_focus_shell();
                    state.set_contents(false);
                    state.set_reorder(false);
                    state.set_book_menu(false);
                    self.refresh_selected();
                }
                None
            }
            #[cfg(target_os = "android")]
            "android-ime-hidden" => {
                crate::android::ui::keyboard_hidden();
                None
            }
            #[cfg(target_os = "android")]
            "android-back" => {
                crate::android::ui::back();
                None
            }
            #[cfg(target_os = "android")]
            "battery-settings" => {
                crate::android::bridge::dispatch(serde_json::json!({"op":"battery-settings"}))
                    .err()
                    .map(|e| Command::Error(format!("{e:#}")))
            }
            #[cfg(target_os = "android")]
            "pick-file" => {
                let owner = super::renew_picker_owner(&w);
                crate::android::bridge::dispatch(serde_json::json!({"op":"pick", "kind":"file", "owner":owner.as_str()}))
                    .err()
                    .map(|e| Command::Error(format!("{e:#}")))
            }
            "resume" => id.map(Command::Resume),
            "sync-invite" => Some(Command::Sync(crate::sync::Request::Invite)),
            "sync-cancel" => Some(Command::Sync(crate::sync::Request::CancelInvite)),
            "sync-now" => Some(Command::Sync(crate::sync::Request::Now)),
            "sync-toggle" => Some(Command::Sync(crate::sync::Request::Running(!state.get_sync_running()))),
            "sync-forget" => Some(Command::Sync(crate::sync::Request::Forget(arg.into()))),
            "sync-pair" => {
                let value = state.get_sync_input().to_string();
                state.set_sync_input("".into());
                Some(Command::Sync(crate::sync::Request::Pair(value)))
            }
            #[cfg(target_os = "android")]
            "sync-scan" => crate::android::bridge::dispatch(serde_json::json!({"op":"sync-scan", "prompt":super::super::text("Наведите камеру на QR-код Carlitos на другом устройстве", "Scan the Carlitos QR code on the other device")}))
                .err().map(|e| Command::Error(format!("{e:#}"))),
            "toggle" => Some(Command::Toggle),
            "previous" => Some(Command::Next(false)),
            "next" => Some(Command::Next(true)),
            "back" => Some(Command::SeekDelta(-i64::from(state.get_seek_step()) * 1000)),
            "forward" => Some(Command::SeekDelta(i64::from(state.get_seek_step()) * 1000)),
            "mute" => Some(Command::Mute),
            "volume-up" => Some(Command::VolumeStep(true)),
            "volume-down" => Some(Command::VolumeStep(false)),
            "rate-up" => Some(Command::RateStep(true)),
            "rate-down" => Some(Command::RateStep(false)),
            "quit" => {
                self.save_window_size();
                Some(Command::Quit)
            }
            "part" => arg
                .split_once(':')
                .and_then(|(id, pos)| Some(Command::Part(id.parse().ok()?, pos.parse().ok()?))),
            "complete" => id.map(Command::Complete),
            "reset-progress" => id.map(Command::ResetProgress),
            "remove-book" => id.map(Command::RemoveBook),
            "select-book"
            | "select-all-books"
            | "deselect-books"
            | "clear-book-selection"
            | "complete-books"
            | "reset-books-progress"
            | "confirm-remove-books"
            | "remove-books" => self.book_selection_action(action, arg),
            "remove-source" | "toggle-source" | "rescan" | "update-source" | "begin-source-edit" | "finish-source-transition" => self.folder_action(action, arg),
            "select-sources"
            | "select-source"
            | "select-all-sources"
            | "deselect-sources"
            | "clear-source-selection"
            | "confirm-remove-sources"
            | "remove-sources" => self.source_selection_action(action, arg),
            "relocate" => id.map(|id| {
                Command::Relocate(id, Location::input(state.get_relocate_path().as_str()))
            }),
            "edit" => state.get_edit_key().parse().ok().map(|id| {
                Command::Edit(
                    id,
                    crate::app::BookEdit {
                        owner: state.get_picker_owner().into(),
                        title: state.get_edit_title().into(),
                        author: state.get_edit_author().into(),
                        cover: (!state.get_edit_cover_path().is_empty())
                            .then(|| state.get_edit_cover_path().to_string()),
                    },
                    vec![],
                )
            }),
            "preview-cover" => Some(self.preview_cover(arg)),
            "reset-cover" => {
                self.reset_cover();
                None
            }
            "begin-edit" => {
                self.begin_edit(id);
                None
            }
            // Resolve the move against the latest order on the controller
            // thread, so queued clicks neither collapse nor overwrite edits.
            "part-up" | "part-down" => arg
                .split(':')
                .next()
                .and_then(|id| id.parse().ok())
                .map(|id| Command::MovePart(id, action == "part-down")),
            "exact-time" => {
                if parse_time(arg, self.playback.duration.unwrap_or(0)).is_some() {
                    state.set_overlay(0);
                }
                Some(Command::Exact(arg.into()))
            }
            "scan" => {
                // Invalidate the old completion immediately, before the new
                // scan's generation has reached this View from the controller.
                super::renew_picker_owner(&w);
                state.set_selected_draft(-1);
                state.set_import_expanded(false);
                Some(Command::Scan(Location::input(arg), state.get_single_book()))
            }
            "cancel-scan" => {
                super::renew_picker_owner(&w);
                state.set_selected_draft(-1);
                state.set_import_expanded(false);
                Some(Command::CancelScan)
            }
            "import" => Some(Command::Import(
                state.get_draft_generation().parse().unwrap_or(0),
                state.get_picker_owner().into(),
            )),
            "draft" => {
                if let Ok(index) = usize::try_from(state.get_selected_draft())
                    && let Some(draft) = self.drafts.get(index)
                {
                    state.set_draft_title(draft.title.clone().into());
                    state.set_draft_author(draft.author.clone().into());
                }
                self.refresh_draft();
                None
            }
            _ => None,
        };
        if let Some(command) = command {
            self.send(command);
        }
    }
}
