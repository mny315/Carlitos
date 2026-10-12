use super::{check, key, snapshot};
use crate::{AppWindow, State, app};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowEvent},
};

pub(super) fn selection_checks(
    window: &AppWindow,
    view: &std::rc::Weak<std::cell::RefCell<app::view::View>>,
) {
    let state = window.global::<State>();
    let keys: Vec<_> = state.get_sources().iter().map(|s| s.key).collect();
    state.invoke_action("select-sources".into(), "".into());
    check(
        state.get_source_selection() && state.get_selected_source_count() == 0,
        "source selection starts empty",
    );
    for key in &keys[..2] {
        state.invoke_action("select-source".into(), key.clone());
    }
    check(
        state.get_selected_source_count() == 2,
        "two independent sources can be selected",
    );
    let mut reordered = app::demo_library();
    reordered.sources.reverse();
    view.upgrade()
        .unwrap()
        .borrow_mut()
        .event(app::Event::Library(Box::new(reordered)));
    check(
        state
            .get_sources()
            .iter()
            .filter(|s| s.selected)
            .all(|s| keys[..2].contains(&s.key))
            && state.get_selected_source_count() == 2,
        "selection follows source IDs across reordered library snapshots",
    );
    snapshot(window, "sources-selection");
    state.invoke_action("confirm-remove-sources".into(), "".into());
    check(
        state.get_overlay() == 1
            && state.get_remove_kind() == "sources"
            && state.get_remove_source_count() == 2,
        "batch removal confirms exactly the selected sources",
    );
    snapshot(window, "sources-remove-confirmation");
    key(window, Key::Escape);
    check(
        state.get_overlay() == 0 && state.get_selected_source_count() == 2,
        "cancel keeps the source selection",
    );
    state.invoke_action("deselect-sources".into(), "".into());
    state.invoke_action("confirm-remove-sources".into(), "".into());
    check(
        state.get_overlay() == 0 && state.get_selected_source_count() == 0,
        "empty selection cannot request removal",
    );
    window.invoke_focus_shell();
    window.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    key(window, "a");
    window.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    check(
        state.get_selected_source_count() as usize == keys.len(),
        "Ctrl+A selects all sources",
    );
    key(window, Key::Delete);
    check(
        state.get_overlay() == 1 && state.get_remove_source_count() as usize == keys.len(),
        "Delete requests batch confirmation",
    );
    key(window, Key::Escape);
    key(window, Key::Escape);
    check(
        !state.get_source_selection() && state.get_selected_source_count() == 0,
        "Escape exits selection after closing confirmation",
    );
    state.invoke_action("select-all-sources".into(), "".into());
    state.set_source_busy(true);
    state.invoke_action("confirm-remove-sources".into(), "".into());
    check(
        state.get_overlay() == 0,
        "source refresh blocks batch removal",
    );
    state.set_source_busy(false);
    check(
        window.invoke_mobile_back() && state.get_page() == 4 && !state.get_source_selection(),
        "Back clears selection before leaving Sources",
    );
    state.invoke_action("select-source".into(), keys[0].clone());
    let mut removed = app::demo_library();
    removed
        .sources
        .retain(|s| s.id.to_string() != keys[0].as_str());
    view.upgrade()
        .unwrap()
        .borrow_mut()
        .event(app::Event::Library(Box::new(removed)));
    check(
        state.get_selected_source_count() == 0 && !state.get_source_selection(),
        "removed sources cannot remain selected",
    );
    view.upgrade()
        .unwrap()
        .borrow_mut()
        .event(app::Event::Library(Box::new(app::demo_library())));
}
