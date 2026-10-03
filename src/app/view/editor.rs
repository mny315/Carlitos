use super::View;
use crate::{AppWindow, State, app::Command, library::Id, source::Location};
use slint::ComponentHandle;
use std::sync::atomic::{AtomicU64, Ordering};

// Workers outlive Android Activities. Never reuse a request ID in another View.
static NEXT_COVER_REQUEST: AtomicU64 = AtomicU64::new(1);
static NEXT_PICKER_OWNER: AtomicU64 = AtomicU64::new(1);

/// A picker belongs to one editing session, even when the same book is reopened.
/// Keep this token across Activity recreation, but never reuse it in a new View.
pub fn renew_picker_owner(window: &AppWindow) -> slint::SharedString {
    let owner: slint::SharedString = NEXT_PICKER_OWNER
        .fetch_add(1, Ordering::Relaxed)
        .to_string()
        .into();
    window.global::<State>().set_picker_owner(owner.clone());
    owner
}

#[cfg(target_os = "android")]
pub(crate) struct SavedEditor {
    key: String,
    title: String,
    author: String,
    cover: String,
    error: String,
    loading: bool,
    request: u64,
}

impl View {
    fn invalidate_cover_request(&mut self) {
        self.cover_request = NEXT_COVER_REQUEST.fetch_add(1, Ordering::Relaxed);
    }

    pub(super) fn preview_cover(&mut self, path: &str) -> Command {
        self.invalidate_cover_request();
        if let Some(window) = self.window.upgrade() {
            renew_picker_owner(&window);
            let state = window.global::<State>();
            state.set_cover_loading(true);
            state.set_edit_cover_error("".into());
        }
        Command::PrepareCover(self.cover_request, Location::input(path))
    }

    pub(super) fn reset_cover(&mut self) {
        self.invalidate_cover_request();
        let Some(window) = self.window.upgrade() else {
            return;
        };
        renew_picker_owner(&window);
        let state = window.global::<State>();
        let cover = state
            .get_edit_key()
            .parse()
            .ok()
            .and_then(|id| self.by_book.get(&id))
            .into_iter()
            .flatten()
            .find_map(|i| {
                self.by_media
                    .get(&self.library.parts[*i].file_id)
                    .and_then(|m| self.library.media[*m].cover.clone())
            });
        state.set_cover_loading(false);
        state.set_edit_cover_error("".into());
        state.set_edit_cover_path(cover.clone().unwrap_or_default().into());
        state.set_edit_cover(self.covers.image(cover.as_deref()));
    }

    pub(super) fn begin_edit(&mut self, id: Option<Id>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(index) = self
            .library
            .books
            .iter()
            .position(|b| Some(b.id) == id.or(self.selected))
        else {
            return;
        };
        self.invalidate_cover_request();
        renew_picker_owner(&window);
        let book = &self.library.books[index];
        let state = window.global::<State>();
        state.set_edit_key(book.id.to_string().into());
        state.set_edit_cover_path(book.cover.clone().unwrap_or_default().into());
        state.set_edit_cover(self.covers.image(book.cover.as_deref()));
        state.set_cover_loading(false);
        state.set_edit_cover_error("".into());
        state.set_edit_title(book.title.clone().into());
        state.set_edit_author(book.author.clone().into());
        state.set_overlay(2);
    }

    pub(super) fn begin_source_edit(&self, id: Option<Id>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let Some(source) = self.library.sources.iter().find(|s| Some(s.id) == id) else {
            return;
        };
        let state = window.global::<State>();
        renew_picker_owner(&window);
        state.set_relocate_key(source.id.to_string().into());
        state.set_relocate_path(
            crate::library::local_path(&source.uri)
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| source.uri.clone())
                .into(),
        );
        state.set_source_error("".into());
        state.set_overlay(5);
    }

    #[cfg(target_os = "android")]
    pub(crate) fn save_editor(&self) -> Option<SavedEditor> {
        let window = self.window.upgrade()?;
        let state = window.global::<State>();
        (state.get_overlay() == 2).then(|| SavedEditor {
            key: state.get_edit_key().into(),
            title: state.get_edit_title().into(),
            author: state.get_edit_author().into(),
            cover: state.get_edit_cover_path().into(),
            error: state.get_edit_cover_error().into(),
            loading: state.get_cover_loading(),
            request: self.cover_request,
        })
    }

    #[cfg(target_os = "android")]
    pub(crate) fn restore_editor(&mut self, saved: SavedEditor) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let id = saved.key.parse::<Id>().ok();
        if !self.library.books.iter().any(|book| Some(book.id) == id) {
            return;
        }
        let state = window.global::<State>();
        // Keep the outstanding worker's ID. Its queued result is delivered
        // after restoration, including when the Activity was absent at completion.
        self.cover_request = saved.request;
        state.set_edit_key(saved.key.into());
        state.set_edit_title(saved.title.into());
        state.set_edit_author(saved.author.into());
        state.set_edit_cover(
            self.covers
                .image((!saved.cover.is_empty()).then_some(saved.cover.as_str())),
        );
        state.set_edit_cover_path(saved.cover.into());
        state.set_cover_loading(saved.loading);
        state.set_edit_cover_error(saved.error.into());
        state.set_overlay(2);
    }
}
