use super::View;
use crate::library::{Book, Id, Progress, Target, format_time};
use crate::{BookItem, PartItem, State, Theme, app::text};
use slint::{ComponentHandle, Model};

impl View {
    pub(super) fn reindex(&mut self) {
        self.book_tags = self.library.book_tags();
        self.by_book.clear();
        self.by_part.clear();
        self.by_media.clear();
        for (i, part) in self.library.parts.iter().enumerate() {
            self.by_book.entry(part.book_id).or_default().push(i);
            self.by_part.insert(part.id, i);
        }
        for parts in self.by_book.values_mut() {
            parts.sort_by_key(|i| self.library.parts[*i].ordinal);
        }
        for (i, file) in self.library.media.iter().enumerate() {
            self.by_media.insert(file.id, i);
        }
    }
    fn summary(&self, book: &Book, progress: Option<&Progress>) -> BookItem {
        let mut total = 0u64;
        let mut elapsed = 0u64;
        let mut found = false;
        let mut known = true;
        for i in self.by_book.get(&book.id).into_iter().flatten() {
            let part = &self.library.parts[*i];
            let file = self
                .by_media
                .get(&part.file_id)
                .map(|i| &self.library.media[*i]);
            let duration = file.and_then(|f| f.duration);
            known &= duration.is_some();
            total = total.saturating_add(duration.unwrap_or(0));
            if let Some(progress) = progress.filter(|p| p.part_id == part.id) {
                elapsed =
                    elapsed.saturating_add(progress.position.min(duration.unwrap_or(u64::MAX)));
                found = true;
            } else if !found {
                elapsed = elapsed.saturating_add(duration.unwrap_or(0));
            }
        }
        if progress.is_none() {
            elapsed = 0;
        }
        let completed = progress.is_some_and(|p| p.completed);
        let fraction = if completed {
            1.
        } else if total > 0 && known {
            (elapsed as f64 / total as f64).clamp(0., 1.) as f32
        } else {
            0.
        };
        let detail = if completed {
            text("Завершена", "Finished").to_owned()
        } else if known && total > 0 {
            format!("{}% · {}", (fraction * 100.) as u32, format_time(total))
        } else {
            crate::i18n::part_count(self.by_book.get(&book.id).map_or(0, Vec::len))
        };
        BookItem {
            key: book.id.to_string().into(),
            title: book.title.clone().into(),
            author: if book.author.is_empty() {
                text("Автор не указан", "Unknown author").into()
            } else {
                book.author.clone().into()
            },
            detail: detail.into(),
            tags: self
                .book_tags
                .get(&book.id)
                .map(|tags| {
                    let mut values = Vec::new();
                    if let Some(year) = tags.year {
                        values.push(year.to_string());
                    }
                    if !tags.genre.is_empty() {
                        values.push(tags.genre.clone());
                    }
                    values.join(" · ")
                })
                .unwrap_or_default()
                .into(),
            progress: fraction,
            cover: slint::Image::default(),
            initials: book
                .title
                .chars()
                .next()
                .unwrap_or('C')
                .to_uppercase()
                .to_string()
                .into(),
            variant: (book.id % 4) as i32,
            started: progress.is_some(),
            completed,
            move_offset: 0.,
        }
    }
    pub(super) fn book_item(&self, id: Id) -> Option<BookItem> {
        self.library.books.iter().find(|b| b.id == id).map(|b| {
            let progress = self.library.progress.iter().find(|p| p.book_id == id);
            let mut item = self.summary(b, progress);
            item.cover = self.covers.image(b.cover.as_deref());
            item
        })
    }
    pub(super) fn refresh_books(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<State>();
        let current = self
            .library
            .session
            .current
            .as_ref()
            .and_then(|Target::Book(id)| {
                self.library.part(*id).map(|part| part.book_id.to_string())
            });
        let current_changed =
            self.books.row_count() > 0 && current.is_some_and(|key| state.get_current().key != key);
        let animate = current_changed
            && state.get_page() == 0
            && !window.global::<Theme>().get_reduced_motion()
            && !self.hidden;
        // A refresh/search visits every book. Resolve progress once instead
        // of scanning all checkpoints twice for each row.
        let progress: std::collections::HashMap<_, _> = self
            .library
            .progress
            .iter()
            .map(|p| (p.book_id, p))
            .collect();
        let sort = match (self.filter, self.settings.library_sort.as_str()) {
            (2, "listening") => "title",
            (_, sort) => sort,
        };
        let rows = self
            .library
            .sorted_books(sort)
            .into_iter()
            .filter(|b| {
                let p = progress.get(&b.id);
                let matches = self.query.is_empty()
                    || b.title.to_lowercase().contains(&self.query)
                    || b.author.to_lowercase().contains(&self.query);
                matches
                    && match self.filter {
                        1 => p.is_some_and(|p| !p.completed),
                        2 => p.is_some_and(|p| p.completed),
                        _ => true,
                    }
            })
            .map(|b| {
                (
                    self.summary(b, progress.get(&b.id).copied()),
                    b.cover.clone(),
                )
            })
            .collect();
        self.books.reset(rows, &state, animate);
        if current_changed && !state.get_books_moving() {
            state.set_books_scroll_y(0.);
        }
    }
    pub(super) fn refresh_selected(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<State>();
        if let Some(id) = self.selected
            && let Some(item) = self.book_item(id)
        {
            state.set_selected(item);
            let mut rows = vec![];
            for (number, i) in self.by_book.get(&id).into_iter().flatten().enumerate() {
                let p = &self.library.parts[*i];
                let f = self
                    .by_media
                    .get(&p.file_id)
                    .map(|i| &self.library.media[*i]);
                rows.push(PartItem {
                    key: format!("{}:0", p.id).into(),
                    title: p.title.clone().into(),
                    detail: f
                        .and_then(|f| f.duration)
                        .map(format_time)
                        .unwrap_or_else(|| {
                            text("Длительность неизвестна", "Unknown duration").into()
                        })
                        .into(),
                    number: format!("{:02}", number + 1).into(),
                    active: false,
                    chapter: false,
                });
                if let Some(f) = f {
                    for chapter in &f.chapters {
                        rows.push(PartItem {
                            key: format!("{}:{}", p.id, chapter.start).into(),
                            title: chapter.title.clone().into(),
                            detail: format_time(chapter.start).into(),
                            number: "·".into(),
                            active: false,
                            chapter: true,
                        });
                    }
                }
            }
            let same_order = self.parts.row_count() == rows.len()
                && self
                    .parts
                    .iter()
                    .zip(&rows)
                    .all(|(old, new)| old.key == new.key && old.chapter == new.chapter);
            if same_order {
                // Volume, settings and metadata updates must not destroy the
                // focused part button or reset its list while the order is stable.
                for (index, mut row) in rows.into_iter().enumerate() {
                    let old = self.parts.row_data(index).unwrap();
                    row.active = old.active;
                    if row != old {
                        self.parts.set_row_data(index, row);
                    }
                }
            } else {
                self.parts.set_vec(rows);
                self.active_part_row = None;
            }
            self.highlight();
        } else if state.get_page() == 1 {
            state.set_page(0);
            self.selected = None;
            self.parts.set_vec(vec![]);
        }
    }
}
