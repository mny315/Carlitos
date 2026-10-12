use super::View;
use crate::{SourceItem, State, app::Command};
use slint::{ComponentHandle, Model};

impl View {
    pub(super) fn refresh_sources(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<State>();
        let had_selection = !self.selected_sources.is_empty();
        self.selected_sources
            .retain(|id| self.library.sources.iter().any(|s| s.id == *id));
        let counts = self.library.source_book_counts();
        let rows: Vec<_> = self
            .library
            .sources
            .iter()
            .map(|s| SourceItem {
                key: s.id.to_string().into(),
                path: crate::source::display_path(&s.uri).into(),
                detail: crate::i18n::book_count(counts.get(&s.id).copied().unwrap_or(0)).into(),
                selected: self.selected_sources.contains(&s.id),
            })
            .collect();
        // Update rows in place so selecting a checkbox or receiving a playback
        // snapshot does not reset the list's scroll position or keyboard focus.
        if rows.len() == self.sources.row_count()
            && rows
                .iter()
                .zip(self.sources.iter())
                .all(|(a, b)| a.key == b.key)
        {
            for (index, row) in rows.into_iter().enumerate() {
                if self.sources.row_data(index).as_ref() != Some(&row) {
                    self.sources.set_row_data(index, row);
                }
            }
        } else {
            self.sources.set_vec(rows);
        }
        state.set_selected_source_count(self.selected_sources.len() as i32);
        if self.library.sources.is_empty() || (had_selection && self.selected_sources.is_empty()) {
            state.set_source_selection(false);
        }
    }

    pub(super) fn source_selection_action(&mut self, action: &str, arg: &str) -> Option<Command> {
        let window = self.window.upgrade()?;
        let state = window.global::<State>();
        match action {
            "select-sources" => state.set_source_selection(!self.library.sources.is_empty()),
            "select-source" => {
                let id = arg.parse().ok()?;
                if !self.library.sources.iter().any(|s| s.id == id) {
                    return None;
                }
                state.set_source_selection(true);
                if !self.selected_sources.insert(id) {
                    self.selected_sources.remove(&id);
                }
            }
            "select-all-sources" => {
                self.selected_sources = self.library.sources.iter().map(|s| s.id).collect();
                state.set_source_selection(!self.selected_sources.is_empty());
            }
            "deselect-sources" => self.selected_sources.clear(),
            "clear-source-selection" => {
                self.selected_sources.clear();
                self.pending_source_removal.clear();
                state.set_source_selection(false);
            }
            "confirm-remove-sources" if !state.get_source_busy() => {
                // Freeze the IDs at confirmation time; a subsequent library
                // refresh must never make the same row index target a new source.
                self.pending_source_removal = self
                    .library
                    .sources
                    .iter()
                    .filter(|s| self.selected_sources.contains(&s.id))
                    .map(|s| s.id)
                    .collect();
                if !self.pending_source_removal.is_empty() {
                    state.set_remove_source_count(self.pending_source_removal.len() as i32);
                    state.set_remove_kind("sources".into());
                    state.set_remove_key("".into());
                    state.set_overlay(1);
                }
            }
            "remove-sources" if !state.get_source_busy() => {
                let ids = std::mem::take(&mut self.pending_source_removal);
                return (!ids.is_empty()).then_some(Command::RemoveSources(ids));
            }
            _ => return None,
        }
        self.refresh_sources();
        None
    }
}
