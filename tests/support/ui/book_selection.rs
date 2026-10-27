use super::check;
use crate::{
    AppWindow, State,
    app::{Command, Event, demo_library, view::View},
};
use slint::{ComponentHandle, Model};

pub(super) fn selection_checks() {
    // Isolate controller messages from the demo used by the other UI checks.
    let window = AppWindow::new().expect("selection test window");
    let (tx, received) = std::sync::mpsc::channel();
    let (view, _covers) = View::new(&window, tx, Default::default());
    view.borrow_mut()
        .event(Event::Library(Box::new(demo_library())));
    let state = window.global::<State>();
    state.invoke_action("select-book".into(), "1".into());
    state.invoke_action("complete-books".into(), "".into());
    let completion = match received.try_recv().expect("complete selection") {
        Command::CompleteBooks(ids, generation) => {
            check(
                ids == [1],
                "batch completion targets its original selection",
            );
            generation
        }
        _ => panic!("expected batch completion"),
    };
    state.invoke_action("clear-book-selection".into(), "".into());
    state.invoke_action("select-book".into(), "2".into());
    state.invoke_action("confirm-remove-books".into(), "".into());
    view.borrow_mut().event(Event::BooksChanged(completion));
    check(
        state.get_book_selection()
            && state.get_selected_book_count() == 1
            && state
                .get_books()
                .iter()
                .any(|book| book.key == "2" && book.selected),
        "an older batch completion preserves a newer book selection",
    );
    state.invoke_action("remove-books".into(), "".into());
    let removal = match received.try_recv().expect("remove selection") {
        Command::RemoveBooks(ids, false, generation) => {
            check(
                ids == [2],
                "an older completion preserves the newer removal confirmation",
            );
            generation
        }
        _ => panic!("expected selected removal"),
    };
    view.borrow_mut().event(Event::BooksChanged(removal));
    check(
        !state.get_book_selection() && state.get_selected_book_count() == 0,
        "a matching batch completion clears its own selection",
    );
    state.invoke_action("select-book".into(), "1".into());
    state.invoke_action("complete-books".into(), "".into());
    let completion = match received.try_recv().expect("second complete selection") {
        Command::CompleteBooks(_, generation) => generation,
        _ => panic!("expected batch completion"),
    };
    state.invoke_action("clear-book-selection".into(), "".into());
    state.invoke_action("select-book".into(), "1".into());
    view.borrow_mut().event(Event::BooksChanged(completion));
    check(
        state.get_book_selection() && state.get_selected_book_count() == 1,
        "reselecting the same book also creates a new selection context",
    );
    state.invoke_action("complete-books".into(), "".into());
    let completion = match received.try_recv().expect("complete before confirmation") {
        Command::CompleteBooks(_, generation) => generation,
        _ => panic!("expected batch completion"),
    };
    state.invoke_action("confirm-remove-books".into(), "".into());
    view.borrow_mut().event(Event::BooksChanged(completion));
    state.invoke_action("remove-books".into(), "".into());
    check(
        matches!(received.try_recv(), Ok(Command::RemoveBooks(ids, false, _)) if ids == [1]),
        "a new confirmation for the same books survives an earlier completion",
    );
}

pub(super) fn restoration_checks() {
    let window = AppWindow::new().expect("book selection snapshot window");
    let (tx, received) = std::sync::mpsc::channel();
    let (view, _covers) = View::new(&window, tx, Default::default());
    let mut library = demo_library();
    view.borrow_mut()
        .event(Event::Library(Box::new(library.clone())));
    let state = window.global::<State>();
    state.invoke_search("".into(), 1);
    state.invoke_action("select-book".into(), "1".into());
    state.invoke_action("select-book".into(), "3".into());
    state.invoke_action("complete-books".into(), "".into());
    let completion = std::iter::from_fn(|| received.try_recv().ok())
        .find_map(|command| match command {
            Command::CompleteBooks(_, generation) => Some(generation),
            _ => None,
        })
        .expect("queued book completion");
    // A remote completion can hide a selected book from Started while its
    // acknowledgement is still queued for the Activity being recreated.
    library
        .progress
        .iter_mut()
        .find(|p| p.book_id == 3)
        .unwrap()
        .completed = true;
    view.borrow_mut()
        .event(Event::Library(Box::new(library.clone())));
    check(
        state.get_selected_book_count() == 2
            && state.get_books().iter().filter(|b| b.selected).count() == 1,
        "a selected book can be hidden by an external progress update",
    );
    let restored = AppWindow::new().expect("restored book selection snapshot window");
    let (target, _target_covers) =
        View::new(&restored, std::sync::mpsc::channel().0, Default::default());
    target.borrow_mut().event(Event::Library(Box::new(library)));
    let state = restored.global::<State>();
    state.invoke_search("".into(), 1);
    view.borrow()
        .test_restore_book_selection(&mut target.borrow_mut());
    check(
        state.get_book_selection() && state.get_selected_book_count() == 2,
        "restoring a book selection retains IDs hidden by its filter",
    );
    target.borrow_mut().event(Event::BooksChanged(completion));
    check(
        !state.get_book_selection() && state.get_selected_book_count() == 0,
        "restored selection accepts its queued completion from the previous view",
    );
}
