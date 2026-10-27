use super::View;
use crate::{State, app::Command, library::Id};
use slint::{ComponentHandle, Model};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_SELECTION: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_generation() -> u64 {
    NEXT_SELECTION.fetch_add(1, Ordering::Relaxed)
}

#[cfg(any(target_os = "android", all(debug_assertions, target_os = "linux")))]
pub(crate) struct SavedBookSelection {
    books: std::collections::BTreeSet<Id>,
    generation: u64,
}

impl View {
    #[cfg(all(debug_assertions, target_os = "linux"))]
    pub fn test_restore_book_selection(&self, target: &mut Self) {
        target.restore_book_selection(self.save_book_selection());
    }

    #[cfg(any(target_os = "android", all(debug_assertions, target_os = "linux")))]
    pub(crate) fn save_book_selection(&self) -> SavedBookSelection {
        SavedBookSelection {
            books: self.selected_books.clone(),
            generation: self.book_selection_generation,
        }
    }

    #[cfg(any(target_os = "android", all(debug_assertions, target_os = "linux")))]
    pub(crate) fn restore_book_selection(&mut self, saved: SavedBookSelection) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let was_empty = saved.books.is_empty();
        self.selected_books = self
            .library
            .books
            .iter()
            .map(|b| b.id)
            .filter(|id| saved.books.contains(id))
            .collect();
        // Retain the context of acknowledgements queued while detached, and
        // retain selected IDs that are temporarily hidden by the active filter.
        self.book_selection_generation = saved.generation;
        window.global::<State>().set_book_selection(
            !self.library.books.is_empty() && (was_empty || !self.selected_books.is_empty()),
        );
        self.refresh_books();
    }

    #[cfg(all(target_os = "android", feature = "android-ui-tests"))]
    pub(crate) fn book_selection_generation(&self) -> u64 {
        self.book_selection_generation
    }

    pub(super) fn book_selection_action(&mut self, action: &str, arg: &str) -> Option<Command> {
        let window = self.window.upgrade()?;
        let state = window.global::<State>();
        let command = match action {
            "select-book" => {
                let id = arg.parse::<Id>().ok()?;
                if !self.library.books.iter().any(|b| b.id == id) {
                    return None;
                }
                self.book_selection_generation = next_generation();
                if !self.selected_books.remove(&id) {
                    self.selected_books.insert(id);
                }
                state.set_book_selection(true);
                state.set_book_menu(false);
                window.invoke_focus_shell();
                None
            }
            "select-all-books" => {
                self.book_selection_generation = next_generation();
                self.selected_books
                    .extend(self.books.iter().filter_map(|b| b.key.parse::<Id>().ok()));
                state.set_book_selection(!self.selected_books.is_empty());
                None
            }
            "deselect-books" => {
                self.book_selection_generation = next_generation();
                self.selected_books.clear();
                None
            }
            "clear-book-selection" => {
                self.book_selection_generation = next_generation();
                self.selected_books.clear();
                self.pending_book_removal.clear();
                self.pending_book_removal_generation = 0;
                state.set_book_selection(false);
                state.set_book_menu(false);
                None
            }
            "confirm-remove-books" => {
                self.pending_book_removal = self.selected_books.iter().copied().collect();
                if !self.pending_book_removal.is_empty() {
                    self.book_selection_generation = next_generation();
                    self.pending_book_removal_generation = self.book_selection_generation;
                    state.set_remove_book_count(self.pending_book_removal.len() as i32);
                    state.set_remove_book_files(false);
                    state.set_remove_kind("books".into());
                    state.set_remove_key("".into());
                    state.set_overlay(1);
                    state.set_book_menu(false);
                }
                None
            }
            "remove-books" => {
                let ids = std::mem::take(&mut self.pending_book_removal);
                (!ids.is_empty()).then(|| {
                    Command::RemoveBooks(
                        ids,
                        state.get_remove_book_files(),
                        self.pending_book_removal_generation,
                    )
                })
            }
            "complete-books" | "reset-books-progress" => {
                let ids: Vec<_> = self.selected_books.iter().copied().collect();
                state.set_book_menu(false);
                (!ids.is_empty()).then(|| {
                    self.book_selection_generation = next_generation();
                    if action == "complete-books" {
                        Command::CompleteBooks(ids, self.book_selection_generation)
                    } else {
                        Command::ResetBooksProgress(ids, self.book_selection_generation)
                    }
                })
            }
            _ => return None,
        };
        self.refresh_books();
        command
    }
}
