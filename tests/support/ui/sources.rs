use super::{check, key, snapshot};
use crate::{AppWindow, State, app};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowEvent},
};

pub(super) fn restoration_checks() {
    use carlitos::library::{Book, Library, Source};
    let library = Library {
        sources: vec![
            Source {
                id: 1,
                uri: "file:///snapshot-source/first".into(),
            },
            Source {
                id: 2,
                uri: "file:///snapshot-source/second".into(),
            },
        ],
        books: vec![
            Book {
                id: 1,
                source_id: 1,
                title: "First".into(),
                author: String::new(),
                cover: None,
            },
            Book {
                id: 2,
                source_id: 2,
                title: "Second".into(),
                author: String::new(),
                cover: None,
            },
        ],
        folder_sources: std::collections::BTreeSet::from([1, 2]),
        ..Default::default()
    };
    let window = AppWindow::new().expect("source selection window");
    let (view, _covers) =
        app::view::View::new(&window, std::sync::mpsc::channel().0, Default::default());
    view.borrow_mut()
        .event(app::Event::Library(Box::new(library.clone())));
    let state = window.global::<State>();
    state.invoke_action("select-source".into(), "1".into());
    state.invoke_action("select-source".into(), "2".into());
    check(
        state.get_selected_source_count() == 2,
        "source snapshot starts with two checked folders",
    );

    let restored = AppWindow::new().expect("restored source selection window");
    let (target, _target_covers) =
        app::view::View::new(&restored, std::sync::mpsc::channel().0, Default::default());
    let mut changed = library.clone();
    changed.sources.push(Source {
        id: 3,
        uri: "file:///snapshot-source/first/new".into(),
    });
    changed.books.push(Book {
        id: 3,
        source_id: 3,
        title: "New".into(),
        author: String::new(),
        cover: None,
    });
    target
        .borrow_mut()
        .event(app::Event::Library(Box::new(changed)));
    view.borrow()
        .test_restore_source_selection(&mut target.borrow_mut());
    let state = restored.global::<State>();
    check(
        state.get_selected_source_count() == 1
            && state
                .get_sources()
                .iter()
                .all(|row| row.selected == (row.key == "2")),
        "restoring source selection never checks a newly discovered group member",
    );
    target
        .borrow_mut()
        .event(app::Event::Library(Box::new(library)));
    view.borrow()
        .test_restore_source_selection(&mut target.borrow_mut());
    check(
        state.get_selected_source_count() == 2,
        "unchanged source groups restore their saved selection",
    );
}

pub(super) fn selection_checks(
    window: &AppWindow,
    view: &std::rc::Weak<std::cell::RefCell<app::view::View>>,
) {
    // Explicitly tracked folders stay independent; legacy book-only siblings
    // are intentionally presented as one recovered collection.
    let original = app::demo_library();
    let mut fixture = original.clone();
    fixture
        .sources
        .retain(|source| fixture.books.iter().any(|book| book.source_id == source.id));
    fixture.folder_sources = fixture.sources.iter().map(|source| source.id).collect();
    view.upgrade()
        .unwrap()
        .borrow_mut()
        .event(app::Event::Library(Box::new(fixture.clone())));
    let state = window.global::<State>();
    let keys: Vec<_> = state.get_sources().iter().map(|s| s.key).collect();
    check(
        keys.len() > 1
            && state
                .get_sources()
                .iter()
                .all(|source| source.detail == "1 книга"),
        "independent single-book folders use the singular form",
    );
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
    let mut reordered = fixture.clone();
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
    let mut enlarged = fixture.clone();
    let root = enlarged
        .sources
        .iter()
        .find(|source| source.id.to_string() == keys[0].as_str())
        .and_then(|source| carlitos::library::local_path(&source.uri))
        .unwrap();
    enlarged.sources.push(carlitos::library::Source {
        id: 999,
        uri: carlitos::library::file_uri(&root.join("new-source")).unwrap(),
    });
    view.upgrade()
        .unwrap()
        .borrow_mut()
        .event(app::Event::Library(Box::new(enlarged)));
    check(
        state.get_selected_source_count() == 1,
        "a changed folder does not silently select newly discovered sources",
    );
    view.upgrade()
        .unwrap()
        .borrow_mut()
        .event(app::Event::Library(Box::new(fixture.clone())));
    check(
        state.get_selected_source_count() == 1,
        "unchecked folder members cannot remain selected across refreshes",
    );
    state.invoke_action("select-source".into(), keys[0].clone());
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
    let mut removed = fixture;
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
        .event(app::Event::Library(Box::new(original)));
}
