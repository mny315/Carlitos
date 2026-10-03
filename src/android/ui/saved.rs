use crate::{State, app};
use slint::{ComponentHandle, Model};
use std::sync::Mutex;

#[derive(Default)]
struct SavedUi {
    page: i32,
    selected: String,
    query: String,
    search_open: bool,
    filter: i32,
    scroll: f32,
    reorder: bool,
    overlay: i32,
    picker_owner: String,
    editor: Option<app::view::SavedEditor>,
    source: Option<SavedSource>,
    exact_time: String,
    import_path: String,
    import_folder_name: String,
    single_book: bool,
    selected_draft: i32,
}
struct SavedSource {
    key: String,
    path: String,
    error: String,
}
static SAVED: Mutex<Option<SavedUi>> = Mutex::new(None);

pub(super) fn save(ui: &app::Ui) {
    let s = ui.window.global::<State>();
    *SAVED.lock().unwrap() = Some(SavedUi {
        page: s.get_page(),
        selected: s.get_selected().key.into(),
        query: s.get_query().into(),
        search_open: s.get_search_open(),
        filter: s.get_filter(),
        scroll: s.get_books_scroll_y(),
        reorder: s.get_reorder(),
        overlay: s.get_overlay(),
        picker_owner: s.get_picker_owner().into(),
        editor: ui.view.borrow().save_editor(),
        source: (s.get_overlay() == 5).then(|| SavedSource {
            key: s.get_relocate_key().into(),
            path: s.get_relocate_path().into(),
            error: s.get_source_error().into(),
        }),
        exact_time: s.get_exact_time().into(),
        import_path: s.get_import_path().into(),
        import_folder_name: s.get_import_folder_name().into(),
        single_book: s.get_single_book(),
        selected_draft: s.get_selected_draft(),
    });
}

/// Restore after the first library snapshot, once book IDs can be resolved.
pub fn restore() {
    let Some(saved) = SAVED.lock().unwrap().take() else {
        return;
    };
    super::WINDOW.with_borrow(|weak| {
        let Some(w) = weak.upgrade() else { return };
        let s = w.global::<State>();
        s.set_query(saved.query.into());
        s.set_search_open(saved.search_open);
        s.set_filter(saved.filter);
        s.invoke_search(s.get_query(), saved.filter);
        if !saved.selected.is_empty() {
            s.invoke_action("open-book".into(), saved.selected.clone().into());
        }
        // Editors below are reopened only if their book/source still exists.
        // Removal confirmations require a fresh action against the current list.
        s.set_overlay(if matches!(saved.overlay, 1 | 2 | 5) {
            0
        } else {
            saved.overlay
        });
        if let Some(editor) = saved.editor {
            super::VIEW.with_borrow(|weak| {
                if let Some(view) = weak.upgrade() {
                    view.borrow_mut().restore_editor(editor);
                }
            });
        }
        if let Some(source) = saved.source
            && s.get_sources()
                .iter()
                .any(|row| row.key.as_str() == source.key)
        {
            s.set_relocate_key(source.key.into());
            s.set_relocate_path(source.path.into());
            s.set_source_error(source.error.into());
            s.set_overlay(5);
        }
        s.set_picker_owner(saved.picker_owner.into());
        // The selection can disappear while the Activity is absent. Do not
        // reopen an empty book page and wait for another library/config update.
        s.set_page(
            if saved.page == 1 && s.get_selected().key.as_str() != saved.selected {
                0
            } else {
                saved.page
            },
        );
        s.set_reorder(saved.reorder);
        s.set_books_scroll_y(saved.scroll);
        s.set_exact_time(saved.exact_time.into());
        s.set_import_path(saved.import_path.into());
        s.set_import_folder_name(saved.import_folder_name.into());
        s.set_single_book(saved.single_book);
        let draft_exists = usize::try_from(saved.selected_draft)
            .is_ok_and(|index| index < s.get_drafts().row_count());
        s.set_selected_draft(if draft_exists {
            saved.selected_draft
        } else {
            -1
        });
        if draft_exists {
            s.invoke_action("draft".into(), saved.selected_draft.to_string().into());
        }
    });
}
