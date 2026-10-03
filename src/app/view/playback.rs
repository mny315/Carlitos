use super::View;
use crate::library::{Target, format_time};
use crate::{State, Theme};
use slint::{ComponentHandle, Model, SharedString};

impl View {
    pub(super) fn refresh_current(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<State>();
        let part = self
            .library
            .session
            .current
            .as_ref()
            .and_then(|Target::Book(id)| self.by_part.get(id))
            .map(|i| &self.library.parts[*i]);
        state.set_active(part.is_some());
        state.set_current(
            part.and_then(|p| self.book_item(p.book_id))
                .unwrap_or_default(),
        );
        let cover = part
            .and_then(|p| self.library.books.iter().find(|book| book.id == p.book_id))
            .and_then(|book| book.cover.as_deref());
        // Keep the displayed palette until the new artwork is decoded. A
        // temporary neutral palette here causes a flash between two books.
        if let Some(palette) = self.covers.palette(cover) {
            w.global::<Theme>()
                .invoke_set_book_colors(palette.dark, palette.light);
        }
        state.set_part_title(
            part.map(|p| SharedString::from(p.title.as_str()))
                .unwrap_or_default(),
        );
        state.set_previous_enabled(self.library.neighbour(false).is_some());
        state.set_next_enabled(self.library.neighbour(true).is_some());
    }
    pub(super) fn refresh_playback(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<State>();
        state.set_playing(self.playback.playing);
        state.set_rate(self.playback.rate as f32);
        state.set_skip_silence(self.playback.skip_silence);
        let loading = matches!(
            self.playback.phase,
            crate::player::Phase::Loading | crate::player::Phase::Seeking
        );
        state.set_playback_loading(loading);
        if !loading {
            state.set_settled_seekable(self.playback.seekable);
        }
        state.set_seekable(self.playback.seekable);
        let duration = self.playback.duration.or_else(|| {
            loading
                .then(|| self.library.active_file().and_then(|file| file.duration))
                .flatten()
        });
        self.refresh_position(self.playback.position, duration);
        if let Some(Target::Book(id)) = self.library.session.current
            && let Some(part) = self.by_part.get(&id).map(|i| &self.library.parts[*i])
            && let Some(item) = self.book_item(part.book_id)
        {
            state.set_current(item.clone());
            self.books.update(item.clone());
            if self.selected == Some(part.book_id) {
                state.set_selected(item);
            }
        }
        self.highlight();
    }
    pub(super) fn refresh_position(&self, position: u64, duration: Option<u64>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<State>();
        state.set_position_text(format_time(position).into());
        state.set_duration_ms(duration.unwrap_or(0) as f32);
        state.set_duration_text(
            duration
                .map(format_time)
                .unwrap_or_else(|| "—:—".into())
                .into(),
        );
        state.set_position(
            duration
                .filter(|d| *d > 0)
                .map_or(0., |d| position as f32 / d as f32),
        );
    }
    pub(super) fn highlight(&mut self) {
        let next = self
            .library
            .active_chapter_key(self.playback.position)
            .and_then(|(part, chapter)| {
                let key = format!("{part}:{}", chapter.unwrap_or(0));
                // The zero chapter and its parent share a seek target, but only
                // the parent should be highlighted outside chapter intervals.
                self.parts
                    .iter()
                    .position(|p| p.key.as_str() == key && p.chapter == chapter.is_some())
            });
        if self.active_part_row != next {
            if let Some(i) = self.active_part_row
                && let Some(mut row) = self.parts.row_data(i)
            {
                row.active = false;
                self.parts.set_row_data(i, row);
            }
            if let Some(i) = next
                && let Some(mut row) = self.parts.row_data(i)
            {
                row.active = true;
                self.parts.set_row_data(i, row);
            }
            self.active_part_row = next;
        }
    }
}
