use super::covers::Covers;
use crate::{BookItem, State};
use slint::{Model, ModelNotify, ModelTracker};
use std::{cell::RefCell, rc::Rc};

pub(super) struct BookModel {
    rows: RefCell<Vec<(BookItem, Option<String>)>>,
    notify: ModelNotify,
    covers: Rc<Covers>,
}
impl Model for BookModel {
    type Data = BookItem;
    fn row_count(&self) -> usize {
        self.rows.borrow().len()
    }
    fn row_data(&self, row: usize) -> Option<BookItem> {
        self.rows.borrow().get(row).map(|(item, path)| {
            let mut item = item.clone();
            item.cover = self.covers.image(path.as_deref());
            item
        })
    }
    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }
}
impl BookModel {
    pub(super) fn new(covers: Rc<Covers>) -> Self {
        Self {
            rows: RefCell::new(vec![]),
            notify: ModelNotify::default(),
            covers,
        }
    }
    pub(super) fn cover_changed(&self, path: &str) {
        let changed: Vec<_> = self
            .rows
            .borrow()
            .iter()
            .enumerate()
            .filter(|(_, (_, p))| p.as_deref() == Some(path))
            .map(|(i, _)| i)
            .collect();
        for row in changed {
            self.notify.row_changed(row);
        }
    }

    pub(super) fn reset(
        &self,
        mut rows: Vec<(BookItem, Option<String>)>,
        state: &State,
        animate: bool,
    ) {
        let previous = self.rows.borrow();
        let same_order = previous.len() == rows.len()
            && previous
                .iter()
                .zip(&rows)
                .all(|(old, new)| old.0.key == new.0.key);
        if same_order {
            for (old, new) in previous.iter().zip(&mut rows) {
                new.0.move_offset = old.0.move_offset;
            }
        } else if animate {
            // Continue from the painted positions if another book was started
            // before the previous movement finished. Offsets use row heights.
            let remaining = 1. - state.get_books_move_progress();
            let positions: std::collections::HashMap<_, _> = previous
                .iter()
                .enumerate()
                .map(|(i, (book, _))| (book.key.clone(), i as f32 + book.move_offset * remaining))
                .collect();
            for (i, (book, _)) in rows.iter_mut().enumerate() {
                book.move_offset = positions.get(&book.key).map_or(0., |old| old - i as f32);
            }
        }
        let changed: Vec<_> = if same_order {
            previous
                .iter()
                .zip(&rows)
                .enumerate()
                .filter_map(|(i, (old, new))| (old != new).then_some(i))
                .collect()
        } else {
            vec![]
        };
        drop(previous);
        *self.rows.borrow_mut() = rows;
        if same_order {
            // Settings/volume refreshes must not destroy the focused book row
            // or reset the list's scroll position when its order is unchanged.
            for row in changed {
                self.notify.row_changed(row);
            }
        } else {
            if animate {
                state.invoke_start_book_move();
            } else {
                state.set_books_moving(false);
            }
            self.notify.reset();
        }
    }
    pub(super) fn update(&self, mut item: BookItem) {
        let index = self
            .rows
            .borrow()
            .iter()
            .position(|(b, _)| b.key == item.key);
        if let Some(index) = index {
            item.move_offset = self.rows.borrow()[index].0.move_offset;
            self.rows.borrow_mut()[index].0 = item;
            self.notify.row_changed(index);
        }
    }
}
