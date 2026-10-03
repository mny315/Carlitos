use super::{check, click, key, sliders::slider_checks, snapshot};
use crate::{
    AppWindow, State,
    app::{Command, Event, view::View},
};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, rc::Rc, time::Duration};

pub(super) fn refresh_checks(window: &AppWindow, view: &Rc<RefCell<View>>) {
    view.borrow_mut()
        .event(Event::Library(Box::new(crate::app::demo_library())));
    let state = window.global::<State>();
    state.invoke_action("open-book".into(), "3".into());
    state.set_contents(true);
    let actions = Rc::new(RefCell::new(Vec::new()));
    let recorded = actions.clone();
    let quit = Rc::downgrade(view);
    // Observe which part Enter addresses. The remaining slider checks only
    // need Quit to reach the controller; they observe their own callbacks.
    state.on_action(move |action, arg| {
        if action == "quit" {
            if let Some(view) = quit.upgrade() {
                view.borrow().send(Command::Quit);
            }
        } else {
            recorded.borrow_mut().push((action, arg));
        }
    });
    let weak = window.as_weak();
    let view = view.clone();
    slint::Timer::single_shot(Duration::from_millis(500), move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let compact = window.global::<State>().get_compact();
        window
            .window()
            .dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(
                    if compact { 180. } else { 500. },
                    if compact { 200. } else { 450. },
                ),
                delta_x: 0.,
                delta_y: -600.,
            });
        slint::Timer::single_shot(Duration::from_millis(500), move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            let width = window
                .window()
                .size()
                .to_logical(window.window().scale_factor())
                .width;
            click(
                &window,
                width - if compact { 42. } else { 56. },
                if compact { 174. } else { 440. },
            );
            key(&window, Key::Return);
            let before = actions.borrow().clone();
            check(
                before.len() == 2 && before[0].0 == "part" && before[0] == before[1],
                "part button responds to click and Enter after scrolling",
            );
            snapshot(&window, "parts-keyboard-focus");
            actions.borrow_mut().clear();
            let mut library = crate::app::demo_library();
            view.borrow_mut()
                .event(Event::Library(Box::new(library.clone())));
            snapshot(&window, "parts-keyboard-refresh");
            key(&window, Key::Return);
            check(
                actions.borrow().as_slice() == &before[1..],
                "unchanged parts keep keyboard focus after a library refresh",
            );
            for part in library.parts.iter_mut().filter(|part| part.book_id == 3) {
                part.title.push_str(" · Updated");
            }
            actions.borrow_mut().clear();
            view.borrow_mut().event(Event::Library(Box::new(library)));
            snapshot(&window, "parts-keyboard-retagged");
            key(&window, Key::Return);
            check(
                actions.borrow().as_slice() == &before[1..],
                "updated part titles preserve the focused playback button",
            );
            slider_checks(&window);
        });
    });
}

pub(super) fn duration_checks(
    window: &AppWindow,
    view: &Rc<std::cell::RefCell<crate::app::view::View>>,
) {
    use crate::app::{Event, audio::Playback};
    let original = crate::app::demo_library();
    let mut library = original.clone();
    library.books.retain(|book| book.id == 1);
    library.parts.retain(|part| part.id == 103);
    library.media.retain(|file| file.id == 103);
    library.media[0].duration = None;
    library.progress.clear();
    let state = window.global::<State>();
    view.borrow_mut().event(Event::Library(Box::new(library)));
    state.invoke_action("open-book".into(), "1".into());
    view.borrow_mut().event(Event::Playback(Playback {
        position: 6_000,
        duration: Some(12_000),
        phase: carlitos::player::Phase::Ready,
        seekable: true,
        ..Default::default()
    }));
    check(
        state.get_current().progress == 0.5 && state.get_selected().progress == 0.5,
        "duration learned during playback updates the book progress",
    );
    check(
        state
            .get_parts()
            .row_data(0)
            .is_some_and(|part| part.detail == "0:12"),
        "duration learned during playback updates the parts list",
    );
    let playback = Playback {
        position: original.session.position,
        duration: original.active_file().and_then(|file| file.duration),
        phase: carlitos::player::Phase::Ready,
        ..Default::default()
    };
    view.borrow_mut().event(Event::Library(Box::new(original)));
    view.borrow_mut().event(Event::Playback(playback));
}
