use super::check;
use crate::{AppWindow, State, Theme};
use slint::{ComponentHandle, Model};
use std::time::Duration;

pub fn system_theme(window: &AppWindow) {
    for second in [1, 3, 6, 9] {
        let weak = window.as_weak();
        slint::Timer::single_shot(Duration::from_secs(second), move || {
            let Some(w) = weak.upgrade() else {
                return;
            };
            let state = w.global::<State>();
            match second {
                1 => {
                    state.set_theme_index(2);
                    state.invoke_settings_changed();
                }
                3 => {
                    check(w.global::<Theme>().get_dark(), "system dark preference");
                    println!("UI THEME DARK");
                }
                6 => {
                    check(
                        !w.global::<Theme>().get_dark(),
                        "portal change updates system theme live",
                    );
                    state.set_theme_index(0);
                    state.invoke_settings_changed();
                    println!("UI THEME MANUAL");
                }
                _ => {
                    check(
                        w.global::<Theme>().get_dark(),
                        "manual theme overrides portal changes",
                    );
                    println!("UI THEME READY");
                }
            }
        });
    }
}
pub fn portal(window: &AppWindow, mode: String) {
    portal_step(window.as_weak(), mode, 0, std::time::Instant::now());
}

fn portal_step(
    weak: slint::Weak<AppWindow>,
    mode: String,
    completed: u8,
    started: std::time::Instant,
) {
    slint::Timer::single_shot(Duration::from_millis(100), move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        let state = w.global::<State>();
        if started.elapsed() >= Duration::from_secs(10) {
            check(false, "portal scenario timed out");
        }
        if state.get_loading() || state.get_picker_busy() {
            portal_step(weak, mode, completed, started);
            return;
        }
        let source = mode == "source";
        if completed < 2 {
            if source {
                state.set_page(4);
                state.set_relocate_key("1".into());
                state.set_relocate_path("/tmp".into());
                state.set_import_path("unchanged import path".into());
                state.set_overlay(5);
                state.invoke_choose_source_folder();
                state.invoke_choose_source_folder();
            } else {
                state.set_page(2);
                state.invoke_choose_folder();
                state.invoke_choose_folder();
            }
            // Rapid clicks, including a different picker, must share one dialog.
            state.invoke_choose_cover();
            check(
                state.get_picker_busy(),
                "picker remains busy until its response",
            );
            portal_step(weak, mode, completed + 1, started);
            return;
        }
        println!("PORTAL NOTICE: {}", state.get_notice());
        check(
            if source {
                state.get_page() == 4
                    && state.get_overlay() == 5
                    && state.get_source_error().is_empty()
                    && state.get_import_path() == "unchanged import path"
                    && state.get_relocate_path().as_str()
                        == std::env::var("CARLITOS_TEST_SOURCE_FOLDER").unwrap()
            } else if mode == "cancel" {
                state.get_notice().is_empty()
            } else {
                state.get_notice().contains("выбор папки")
            },
            "portal selection/cancellation/error preserves the correct page and fields",
        );
        println!("UI PORTAL READY");
    });
}
pub fn library_tab(window: &AppWindow, expected: i32) {
    library_tab_step(window.as_weak(), expected, std::time::Instant::now());
}
fn library_tab_step(weak: slint::Weak<AppWindow>, expected: i32, started: std::time::Instant) {
    slint::Timer::single_shot(Duration::from_millis(100), move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        let s = w.global::<State>();
        if started.elapsed() >= Duration::from_secs(12) {
            check(false, "library tab restore timed out");
        }
        if s.get_loading() {
            library_tab_step(weak, expected, started);
            return;
        }
        check(
            s.get_filter() == expected,
            "saved library tab is selected on startup",
        );
        let books = s.get_books();
        check(
            books.row_count() > 0,
            "restored tab contains the imported book",
        );
        check(
            if expected == 1 {
                (0..books.row_count()).all(|i| {
                    let book = books.row_data(i).unwrap();
                    book.started && !book.completed
                })
            } else {
                books.row_count() == s.get_book_count() as usize
            },
            "restored tab filters the actual library model",
        );
        if expected == 1 {
            s.set_filter(0);
            s.invoke_search("".into(), 0);
            s.set_filter(2);
            s.invoke_search("".into(), 2);
            s.invoke_settings_changed();
            check(
                s.get_filter() == 2,
                "saving settings does not leave the completed tab",
            );
        }
        println!("UI TAB RESTORED {expected}");
    });
}
