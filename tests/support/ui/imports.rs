use super::check;
use crate::{AppWindow, State};
use slint::{ComponentHandle, Model};
use std::rc::Rc;

pub(super) fn draft_checks(
    window: &AppWindow,
    view: &Rc<std::cell::RefCell<crate::app::view::View>>,
) {
    view.borrow_mut().event(crate::app::Event::Drafts(
        vec![carlitos::storage::Draft {
            root_uri: "file:///import-preview".into(),
            title: "Draft title".into(),
            author: "Draft author".into(),
            files: vec![carlitos::library::Media::default()],
            include: true,
        }],
        vec![],
    ));
    let state = window.global::<State>();
    let language = state.get_language_index();
    for (index, detail) in [(1, "1 часть"), (2, "1 part")] {
        state.set_language_index(index);
        state.invoke_settings_changed();
        check(
            state
                .get_drafts()
                .row_data(0)
                .is_some_and(|draft| draft.detail == detail),
            "import part count follows the selected language and singular form",
        );
    }
    state.set_language_index(language);
    state.invoke_settings_changed();
    state.set_selected_draft(0);
    state.invoke_action("draft".into(), "0".into());
    check(
        state.get_draft_title() == "Draft title" && state.get_draft_author() == "Draft author",
        "opening an import draft initializes its own fields",
    );
    state.set_page(0);
    // The import preview remains open when navigating away to edit a book.
    state.invoke_action("begin-edit".into(), "1".into());
    state.set_edit_author("Library book author".into());
    state.set_overlay(0);
    state.set_page(2);
    check(
        state.get_draft_title() == "Draft title" && state.get_draft_author() == "Draft author",
        "editing a library book does not overwrite import draft fields",
    );
    let draft = carlitos::storage::Draft {
        root_uri: "file:///first".into(),
        title: "First draft".into(),
        author: String::new(),
        files: vec![carlitos::library::Media::default()],
        include: true,
    };
    let second = carlitos::storage::Draft {
        root_uri: "file:///second".into(),
        title: "Second draft".into(),
        ..draft.clone()
    };
    view.borrow_mut().event(crate::app::Event::Drafts(
        vec![draft.clone(), second],
        vec![],
    ));
    state.set_selected_draft(1);
    state.invoke_action("draft".into(), "1".into());
    view.borrow_mut()
        .event(crate::app::Event::Drafts(vec![draft], vec![]));
    check(
        state.get_selected_draft() == -1 && state.get_draft_files().row_count() == 0,
        "a refreshed draft list closes an editor whose entry no longer exists",
    );
    view.borrow_mut()
        .event(crate::app::Event::Drafts(vec![], vec![]));
}
