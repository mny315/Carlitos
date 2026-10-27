use super::check;
use crate::{AppWindow, State};
use slint::{ComponentHandle, Model};
use std::rc::Rc;

pub(super) fn completion_checks() {
    use crate::app::{Command, Event, view::View};
    let window = AppWindow::new().expect("import completion window");
    let (tx, received) = std::sync::mpsc::channel();
    let (view, _covers) = View::new(&window, tx, Default::default());
    let state = window.global::<State>();
    // A standalone Android file import need not have scanned any preview yet.
    state.set_page(4);
    state.set_picker_owner("first-file".into());
    view.borrow_mut()
        .event(Event::Imported(0, "first-file".into()));
    check(
        state.get_page() == 0,
        "a first file import can finish before any folder scan",
    );
    let draft = carlitos::storage::Draft {
        root_uri: "file:///import-completion".into(),
        title: "Original preview".into(),
        author: String::new(),
        files: vec![carlitos::library::Media::default()],
        include: true,
    };
    state.set_page(2);
    view.borrow_mut()
        .event(Event::Drafts(10, vec![draft.clone()], vec![]));
    state.invoke_action("import".into(), "".into());
    let owner = match received.try_recv().expect("import request") {
        Command::Import(10, owner) => owner,
        _ => panic!("expected import with preview generation"),
    };
    state.invoke_action("scan".into(), "/next-preview".into());
    check(
        matches!(received.try_recv(), Ok(Command::Scan(..))),
        "another preview is requested before the old import completes",
    );
    view.borrow_mut().event(Event::Imported(10, owner.clone()));
    check(
        state.get_page() == 2,
        "an old import completion preserves a newly requested preview",
    );
    view.borrow_mut()
        .event(Event::Drafts(11, vec![draft], vec![]));
    state.set_selected_draft(0);
    state.invoke_action("draft".into(), "0".into());
    let current_owner = state.get_picker_owner();
    // Android can restore the previous owner's saved UI after receiving a
    // newer preview snapshot produced while its Activity was absent.
    state.set_picker_owner(owner.clone().into());
    view.borrow_mut().event(Event::Imported(10, owner));
    check(
        state.get_page() == 2 && state.get_selected_draft() == 0,
        "an old import completion preserves a newer preview editor",
    );
    state.set_picker_owner(current_owner);
    state.invoke_action("import".into(), "".into());
    let owner = match received.try_recv().expect("current import request") {
        Command::Import(11, owner) => owner,
        _ => panic!("expected current preview import"),
    };
    view.borrow_mut().event(Event::Drafts(11, vec![], vec![]));
    view.borrow_mut().event(Event::Imported(11, owner));
    check(
        state.get_page() == 0 && state.get_selected_draft() == -1,
        "a matching import completion returns to the library",
    );
}

pub(super) fn draft_checks(
    window: &AppWindow,
    view: &Rc<std::cell::RefCell<crate::app::view::View>>,
) {
    view.borrow_mut().event(crate::app::Event::Drafts(
        1,
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
        2,
        vec![draft.clone(), second.clone()],
        vec![],
    ));
    state.set_selected_draft(1);
    state.invoke_action("draft".into(), "1".into());
    view.borrow_mut()
        .event(crate::app::Event::Drafts(2, vec![draft], vec![]));
    check(
        state.get_selected_draft() == -1 && state.get_draft_files().row_count() == 0,
        "a refreshed draft list closes an editor whose entry no longer exists",
    );
    state.set_selected_draft(0);
    state.invoke_action("draft".into(), "0".into());
    view.borrow_mut()
        .event(crate::app::Event::Drafts(3, vec![second], vec![]));
    check(
        state.get_selected_draft() == -1 && state.get_draft_files().row_count() == 0,
        "a new scan closes the old draft editor even when its row index still exists",
    );
    view.borrow_mut()
        .event(crate::app::Event::Drafts(3, vec![], vec![]));
}
