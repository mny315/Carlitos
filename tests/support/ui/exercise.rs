use super::check;
use crate::{AppWindow, State};
use slint::{ComponentHandle, Model};
use std::time::Duration;

/// A real UI import/playback path on an explicitly isolated test library.
pub fn exercise(window: &AppWindow, folder: String) {
    exercise_step(window.as_weak(), folder, 0, std::time::Instant::now(), None);
}
fn exercise_step(
    weak: slint::Weak<AppWindow>,
    folder: String,
    mut stage: usize,
    started: std::time::Instant,
    mut reordered: Option<slint::SharedString>,
) {
    slint::Timer::single_shot(Duration::from_millis(250), move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        let s = w.global::<State>();
        if started.elapsed() >= Duration::from_secs(45) {
            check(false, "UI scenario timed out");
        }
        match stage {
            0 if !s.get_loading() => {
                s.set_page(2);
                s.set_import_path(folder.clone().into());
                s.invoke_action("scan".into(), folder.clone().into());
                stage = 1;
            }
            1 if !s.get_scanning() && s.get_drafts().row_count() > 0 => {
                check(
                    s.get_book_count() == 0,
                    "scan preview has not changed the library",
                );
                s.set_selected_draft(0);
                s.invoke_action("draft".into(), "0".into());
                reordered = Some(s.get_draft_files().row_data(2).expect("preview file").title);
                s.invoke_draft_file_action(2, "up".into());
                s.invoke_draft_file_action(2, "up".into());
                stage = 10;
            }
            10 if s
                .get_draft_files()
                .row_data(0)
                .is_some_and(|file| Some(&file.title) == reordered.as_ref()) =>
            {
                check(true, "rapid preview moves keep the selected file");
                s.invoke_draft_file_action(0, "down".into());
                s.invoke_draft_file_action(0, "down".into());
                stage = 11;
            }
            11 if s
                .get_draft_files()
                .row_data(2)
                .is_some_and(|file| Some(&file.title) == reordered.as_ref()) =>
            {
                check(true, "rapid preview reverse moves restore import order");
                reordered = None;
                s.set_selected_draft(-1);
                s.invoke_action("import".into(), "".into());
                stage = 2;
            }
            2 if s.get_book_count() > 0 => {
                check(
                    !s.get_notice().is_empty(),
                    "import completion shows a notification",
                );
                stage = 12;
            }
            12 if s.get_notice().is_empty() => {
                check(true, "import notification dismisses itself");
                let book = s.get_books().row_data(0).expect("imported book");
                s.invoke_action("open-book".into(), book.key.clone());
                s.invoke_set_volume(0.02);
                s.invoke_action("resume".into(), book.key);
                stage = 3;
            }
            3 if s.get_seekable() && s.get_playing() => {
                s.invoke_seek(0.25);
                stage = 4;
            }
            4 if s.get_position() >= 0.24 => {
                s.invoke_action("toggle".into(), "".into());
                stage = 5;
            }
            5 if !s.get_playing() => {
                check(
                    s.get_parts().row_count() >= 3,
                    "imported book contents visible",
                );
                let path = std::path::Path::new(&folder).join("my-cover.png");
                image::RgbImage::from_pixel(160, 240, image::Rgb([41, 74, 91]))
                    .save(&path)
                    .expect("custom cover fixture");
                s.invoke_action("begin-edit".into(), s.get_selected().key);
                s.invoke_action("preview-cover".into(), path.display().to_string().into());
                stage = 6;
            }
            6 if !s.get_cover_loading() && s.get_edit_cover().size().width > 0 => {
                check(
                    s.get_edit_cover_error().is_empty(),
                    "custom cover preview is decoded",
                );
                check(
                    std::path::Path::new(s.get_edit_cover_path().as_str()).is_file(),
                    "custom cover has a managed copy",
                );
                std::fs::remove_file(std::path::Path::new(&folder).join("my-cover.png"))
                    .expect("remove original fixture");
                s.set_sort_index(3);
                s.invoke_settings_changed();
                s.invoke_action("edit".into(), "".into());
                stage = 7;
            }
            7 if s.get_overlay() == 0 && s.get_selected().cover.size().width > 0 => {
                check(
                    s.get_current().cover.size().width > 0,
                    "saved custom cover reaches book and player",
                );
                let last = s.get_parts().row_data(2).unwrap().key;
                s.invoke_action("part-up".into(), last.clone());
                s.invoke_action("part-up".into(), last.clone());
                reordered = Some(last);
                stage = 8;
            }
            8 if s.get_parts().row_data(0).map(|p| p.key) == reordered => {
                check(true, "rapid part moves are applied in sequence");
                // Restore the fixture order for the desktop playback checks.
                let first = reordered.take().unwrap();
                s.invoke_action("part-down".into(), first.clone());
                s.invoke_action("part-down".into(), first.clone());
                reordered = Some(first);
                stage = 9;
            }
            9 if s.get_parts().row_data(2).map(|p| p.key) == reordered => {
                check(true, "rapid reverse moves restore the original order");
                s.set_page(0);
                s.set_filter(1);
                s.invoke_search("".into(), 1);
                s.set_filter(2);
                s.invoke_search("".into(), 2);
                s.invoke_settings_changed();
                println!("UI SCENARIO READY");
                return;
            }
            _ => {}
        }
        exercise_step(weak, folder, stage, started, reordered);
    });
}
