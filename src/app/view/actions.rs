use super::View;
use crate::app::Command;
use crate::library::{Id, format_time, local_path, parse_time};
use crate::source::Location;
use crate::{AppWindow, State};
use slint::ComponentHandle;
use std::{cell::RefCell, rc::Rc};

impl View {
    pub(super) fn bind(view: &Rc<RefCell<Self>>, window: &AppWindow) {
        let state = window.global::<State>();
        let weak = Rc::downgrade(view);
        state.on_search(move |q, f| {
            if let Some(v) = weak.upgrade() {
                let mut v = v.borrow_mut();
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
        state.on_draft_changed(move |index, title, author, included| {
            if index >= 0 {
                let _ = tx.send(Command::DraftEdit(
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
                crate::android::bridge::dispatch(serde_json::json!({"op":"pick", "kind":"file"}))
                    .err()
                    .map(|e| Command::Error(format!("{e:#}")))
            }
            "resume" => id.map(Command::Resume),
            "toggle" => Some(Command::Toggle),
            "previous" => Some(Command::Next(false)),
            "next" => Some(Command::Next(true)),
            "back" => Some(Command::SeekDelta(-i64::from(state.get_seek_step()) * 1000)),
            "forward" => Some(Command::SeekDelta(i64::from(state.get_seek_step()) * 1000)),
            "mute" => Some(Command::Mute),
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
            "remove-source" => id.map(Command::RemoveSource),
            "rescan" => id.map(Command::Rescan),
            "update-source" => id.and_then(|id| {
                let source = self.library.sources.iter().find(|s| s.id == id)?;
                let mut path = Location::input(state.get_relocate_path().as_str());
                if let Location::File(local) = &mut path {
                    *local = dunce::canonicalize(&*local).unwrap_or(local.clone());
                }
                let same = match &path {
                    Location::File(local) => local_path(&source.uri).as_ref() == Some(local),
                    Location::Document(uri) => &source.uri == uri,
                };
                Some(if same {
                    Command::Rescan(id)
                } else {
                    Command::Relocate(id, path)
                })
            }),
            "relocate" => id.map(|id| {
                Command::Relocate(id, Location::input(state.get_relocate_path().as_str()))
            }),
            "edit" => state.get_edit_key().parse().ok().map(|id| {
                Command::Edit(
                    id,
                    state.get_edit_title().into(),
                    state.get_edit_author().into(),
                    vec![],
                    (!state.get_edit_cover_path().is_empty())
                        .then(|| state.get_edit_cover_path().to_string()),
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
            "begin-source-edit" => {
                self.begin_source_edit(id);
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
                state.set_selected_draft(-1);
                Some(Command::Scan(Location::input(arg), state.get_single_book()))
            }
            "cancel-scan" => {
                state.set_selected_draft(-1);
                Some(Command::CancelScan)
            }
            "import" => Some(Command::Import),
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
