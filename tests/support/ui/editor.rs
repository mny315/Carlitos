use super::check;
use crate::{
    AppWindow, State,
    app::{Command, Event, demo_library, view::View},
};
use slint::ComponentHandle;

pub(super) fn completion_checks() {
    let window = AppWindow::new().expect("editor completion window");
    let (tx, received) = std::sync::mpsc::channel();
    let (view, _covers) = View::new(&window, tx, Default::default());
    view.borrow_mut()
        .event(Event::Library(Box::new(demo_library())));
    let state = window.global::<State>();
    state.invoke_action("begin-edit".into(), "1".into());
    let owner = state.get_picker_owner();
    state.set_edit_title("First saved title".into());
    state.invoke_action("edit".into(), "".into());
    let submitted = match received.try_recv().expect("submitted edit") {
        Command::Edit(1, edit, _) => edit,
        _ => panic!("expected editor save"),
    };
    state.set_overlay(0);
    state.invoke_action("begin-edit".into(), "1".into());
    state.set_edit_title("New unsaved title".into());
    check(
        state.get_picker_owner() != owner,
        "reopened editor has a fresh owner",
    );
    view.borrow_mut().event(Event::BookEdited(1, submitted));
    check(
        state.get_overlay() == 2 && state.get_edit_title() == "New unsaved title",
        "an old save completion preserves a reopened editor for the same book",
    );
    state.invoke_action("edit".into(), "".into());
    let submitted = match received.try_recv().expect("new editor save") {
        Command::Edit(1, edit, _) => edit,
        _ => panic!("expected new editor save"),
    };
    state.set_edit_title("Further unsaved title".into());
    view.borrow_mut().event(Event::BookEdited(1, submitted));
    check(
        state.get_overlay() == 2 && state.get_edit_title() == "Further unsaved title",
        "a save completion preserves text entered after submitting the same editor",
    );
    state.invoke_action("edit".into(), "".into());
    let submitted = match received.try_recv().expect("final editor save") {
        Command::Edit(1, edit, _) => edit,
        _ => panic!("expected final editor save"),
    };
    view.borrow_mut().event(Event::BookEdited(1, submitted));
    check(
        state.get_overlay() == 0,
        "a matching save completion closes its editor",
    );
}
