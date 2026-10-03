use super::super::{
    check, click, click_book_menu, first_library_menu, key, motion_snapshot,
    parts::{duration_checks, refresh_checks},
    snapshot,
};
use crate::{AppWindow, State};
use slint::{ComponentHandle, Model, platform::Key};

pub(super) fn step(
    w: AppWindow,
    view: &std::rc::Weak<std::cell::RefCell<crate::app::view::View>>,
    index: usize,
) {
    let s = w.global::<State>();
    match index {
        0 => {
            check(
                !s.get_selected().started,
                "unplayed book has no resume state",
            );
            snapshot(&w, "book-unplayed");
            let size = w.window().size().to_logical(w.window().scale_factor());
            click(
                &w,
                size.width - 48.,
                if s.get_compact() { 81. } else { 47. },
            );
            motion_snapshot(&w, "menu-transition");
        }
        1 => {
            check(
                s.get_overlay() == 2 && s.get_page() == 1 && s.get_edit_key() == "2",
                "ellipsis inside a book opens its editor directly",
            );
            snapshot(&w, "book-editor-from-book");
            key(&w, Key::Escape);
            check(s.get_overlay() == 0, "Escape dismisses the book editor");
            s.invoke_navigate(0);
            s.set_query("Невидимые".into());
            s.invoke_search(s.get_query(), 0);
        }
        2 => {
            first_library_menu(&w);
            check(
                s.get_book_menu()
                    && s.get_page() == 0
                    && s.get_overlay() == 0
                    && s.get_menu_book().key == "2",
                "list ellipsis opens actions for its own book without opening the editor",
            );
            motion_snapshot(&w, "library-menu-transition");
        }
        3 => {
            snapshot(&w, "library-book-menu");
            key(&w, Key::Escape);
            check(!s.get_book_menu(), "Escape dismisses library book actions");
            first_library_menu(&w);
            click_book_menu(&w, "remove");
            check(
                s.get_overlay() == 1 && s.get_remove_key() == "2",
                "Remove confirms the book targeted by the library menu",
            );
            key(&w, Key::Escape);
            s.invoke_action("open-book".into(), "4".into());
            check(
                s.get_selected().completed,
                "finished book retains replay state",
            );
            snapshot(&w, "book-completed");
            s.set_page(0);
            s.set_query("".into());
            s.invoke_search("".into(), 0);
            s.set_sort_index(2);
            s.invoke_settings_changed();
        }
        4 => {
            check(
                s.get_books().row_data(0).unwrap().key == s.get_current().key
                    && s.get_books().row_data(1).unwrap().key == "6",
                "author sorting keeps the current book first and sorts the rest",
            );
            s.set_sort_index(0);
            s.invoke_settings_changed();
        }
        5 => {
            let books = s.get_books();
            check(
                books.row_data(0).unwrap().started && !books.row_data(0).unwrap().completed,
                "default sorting starts with listening books",
            );
            s.set_query("Невидимые".into());
            s.invoke_search(s.get_query(), 0);
        }
        6 => {
            first_library_menu(&w);
            click_book_menu(&w, "edit");
            check(
                s.get_page() == 1
                    && s.get_overlay() == 2
                    && s.get_edit_key() == "2"
                    && s.get_selected().key == "2"
                    && !s.get_book_menu(),
                "Edit from the library menu opens the matching book and its editor",
            );
            motion_snapshot(&w, "book-editor-opening");
        }
        7 => {
            snapshot(&w, "book-editor");
            key(&w, Key::Escape);
            motion_snapshot(&w, "book-editor-closing");
            s.invoke_action("open-book".into(), s.get_edit_key());
            s.set_reorder(true);
            s.set_contents(true);
        }
        8 => {
            snapshot(&w, "book-reorder");
            s.set_page(0);
            s.set_filter(2);
            s.set_query("".into());
            s.invoke_search("".into(), 2);
        }
        9 => {
            let books = s.get_books();
            check(
                books.row_count() > 0
                    && (0..books.row_count()).all(|i| books.row_data(i).unwrap().completed),
                "completed filter contains only finished books with default sorting",
            );
            snapshot(&w, "library-completed");
            check(
                s.get_sort_index() == 0 && s.get_completed_sort_index() == 0,
                "completed books use title order without changing the library preference",
            );
            s.set_filter(0);
            s.invoke_search("".into(), 0);
            let first = s.get_books().row_data(0).unwrap();
            check(
                first.started && !first.completed,
                "returning to all books preserves started-first ordering",
            );
            s.set_filter(2);
            s.set_query("no matching completed book".into());
            s.invoke_search(s.get_query(), 2);
        }
        10 => {
            check(
                s.get_books().row_count() == 0,
                "completed search can be empty",
            );
            snapshot(&w, "library-completed-empty");
            s.set_query("".into());
            s.invoke_search("".into(), 2);
            let (x, y) = if s.get_compact() {
                (200., 176.)
            } else {
                (426., 230.)
            };
            click(&w, x, y);
        }
        11 => {
            snapshot(&w, "library-completed-sort-menu");
            key(&w, Key::UpArrow);
            check(
                s.get_sort_index() == 1,
                "first completed sort option is title",
            );
            key(&w, Key::DownArrow);
            check(
                s.get_sort_index() == 2,
                "completed author option maps correctly",
            );
            key(&w, Key::DownArrow);
            check(
                s.get_sort_index() == 3,
                "completed year option maps correctly",
            );
            key(&w, Key::DownArrow);
            check(
                s.get_sort_index() == 4,
                "completed genre option maps correctly",
            );
            key(&w, Key::DownArrow);
            check(
                s.get_sort_index() == 4,
                "completed sorting has only four options",
            );
            key(&w, Key::Escape);
            s.set_filter(0);
            s.invoke_search("".into(), 0);
        }
        12 => {
            check(
                s.get_sort_index() == 4,
                "selected genre order survives returning to the full library",
            );
            snapshot(&w, "library-sort-after-completed");
            w.invoke_focus_search();
            // Resolve the layout after moving focus, as a rendered
            // frame does before the next real keyboard event.
            snapshot(&w, "library-keyboard-search");
            for _ in 0..if s.get_compact() { 6 } else { 5 } {
                key(&w, Key::Tab);
            }
            snapshot(&w, "library-keyboard-focus");
            if let Some(view) = view.upgrade() {
                view.borrow_mut().event(crate::app::Event::Library(Box::new(
                    crate::app::demo_library(),
                )));
            }
            snapshot(&w, "library-keyboard-refresh");
            key(&w, Key::Return);
            check(
                s.get_page() == 1 && s.get_overlay() == 0,
                "a library row opens its book from the keyboard",
            );
            s.set_page(4);
        }
        13 => {
            check(
                s.get_sources().row_data(0).unwrap().detail == "6 книг",
                "parent source includes nested books",
            );
            check(
                s.get_sources().row_data(1).unwrap().detail == "1 книга",
                "single-book sources use the singular form",
            );
            snapshot(&w, "sources");
            let (x, y) = if s.get_compact() {
                (80., 181.)
            } else {
                (285., 190.)
            };
            click(&w, x, y);
        }
        14 => {
            check(
                s.get_overlay() == 5 && s.get_page() == 4,
                "source update opens in place",
            );
            check(
                s.get_relocate_path() == "/home/you/Audiobooks",
                "source editor starts with the saved path",
            );
            snapshot(&w, "source-editor");
            key(&w, Key::Escape);
        }
        15 => {
            s.invoke_navigate(0);
            s.set_filter(1);
            s.set_query("Левая".into());
            s.invoke_search(s.get_query(), 1);
        }
        16 => {
            first_library_menu(&w);
            check(
                s.get_menu_book().key == "3" && s.get_menu_book().started,
                "started book offers a progress reset",
            );
            motion_snapshot(&w, "library-reset-progress-menu");
        }
        17 => {
            snapshot(&w, "library-reset-progress-menu");
            click_book_menu(&w, "reset");
        }
        18 => {
            check(
                !s.get_book_menu(),
                "mark-as-unstarted button closes the book menu",
            );
            check(s.get_page() == 0, "progress reset keeps the library open");
            check(
                s.get_books().iter().all(|b| b.key != "3"),
                "reset book leaves the started filter",
            );
            check(
                s.get_current().key == "1",
                "resetting another book preserves the current book",
            );
            s.invoke_action("open-book".into(), "3".into());
            check(
                !s.get_selected().started && s.get_selected().progress == 0.,
                "mark-as-unstarted clears the menu book's progress",
            );
            check(
                s.get_parts().row_count() == 20,
                "progress reset preserves the book's parts",
            );
            if let Some(view) = view.upgrade() {
                duration_checks(&w, &view);
                slint::spawn_local(async move {
                    super::super::books::playback_checks(&w, &view).await;
                    refresh_checks(&w, &view);
                })
                .expect("book animation checks");
            }
        }
        _ => unreachable!(),
    }
}
