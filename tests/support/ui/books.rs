use super::{check, snapshot};
use crate::{AppWindow, State, Theme, app};
use carlitos::{library::Target, player::Phase};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, rc::Rc, time::Duration};

async fn wait(ms: u64) {
    let (tx, rx) = async_channel::bounded(1);
    slint::Timer::single_shot(Duration::from_millis(ms), move || {
        let _ = tx.try_send(());
    });
    rx.recv().await.expect("book animation timer");
}

fn first_row(window: &AppWindow) -> Vec<u8> {
    let pixels = window.window().take_snapshot().expect("library frame");
    let compact = window.global::<State>().get_compact();
    let scale = window.window().scale_factor();
    let scaled = |n: f32| (n * scale).round() as usize;
    let left = scaled(if compact { 12. } else { 218. });
    let top = scaled(if compact { 202. } else { 264. });
    let width = pixels.width() as usize;
    let mut result = Vec::new();
    for y in top..top + scaled(84.) {
        result.extend_from_slice(
            &pixels.as_bytes()[(y * width + left) * 4..((y + 1) * width - scaled(28.)) * 4],
        );
    }
    result
}

pub async fn playback_checks(window: &AppWindow, view: &Rc<RefCell<app::view::View>>) {
    let state = window.global::<State>();
    let theme = window.global::<Theme>();
    state.set_page(0);
    state.set_query("".into());
    state.set_filter(0);
    state.invoke_search("".into(), 0);
    state.set_sort_index(1);
    theme.set_reduced_motion(false);
    state.invoke_settings_changed();
    let mut library = app::demo_library();
    view.borrow_mut()
        .event(app::Event::Library(Box::new(library.clone())));
    let playback = |playing| {
        app::Event::Playback(app::audio::Playback {
            playing,
            phase: Phase::Ready,
            position: 120_000,
            duration: Some(2_091_000),
            seekable: true,
            ..Default::default()
        })
    };
    view.borrow_mut().event(playback(false));
    wait(750).await;
    let before = first_row(window);
    library.session.current = Some(Target::Book(203));
    view.borrow_mut()
        .event(app::Event::Library(Box::new(library.clone())));
    view.borrow_mut().event(playback(false));
    check(
        state.get_books().row_data(0).unwrap().key == "2" && state.get_books_moving(),
        "switching books pins the new book and starts its movement",
    );
    let mut frames = Vec::new();
    for _ in 0..4 {
        wait(55).await;
        frames.push(first_row(window));
    }
    snapshot(window, "library-book-moving");
    wait(300).await;
    let after = first_row(window);
    check(
        before != after
            && frames.windows(2).filter(|f| f[0] != f[1]).count() >= 2
            && frames.iter().any(|f| *f != before && *f != after),
        "book movement renders intermediate positions before settling",
    );
    check(
        !state.get_books_moving(),
        "book movement stops after settling",
    );
    view.borrow_mut().event(playback(true));
    wait(100).await;
    let playing = first_row(window);
    wait(150).await;
    check(
        playing != first_row(window),
        "playing book has an animated indicator",
    );
    snapshot(window, "library-book-playing");
    view.borrow_mut().event(playback(false));
    wait(200).await;
    let paused = first_row(window);
    wait(150).await;
    check(
        paused == first_row(window) && state.get_books().row_data(0).unwrap().key == "2",
        "pause stops the indicator and keeps the book first",
    );
    theme.set_reduced_motion(true);
    view.borrow_mut().event(playback(true));
    wait(100).await;
    let still = first_row(window);
    wait(150).await;
    check(
        still == first_row(window),
        "reduced motion keeps the playing indicator static",
    );
    state.set_books_scroll_y(-84.);
    wait(50).await;
    library.session.current = Some(Target::Book(303));
    view.borrow_mut()
        .event(app::Event::Library(Box::new(library.clone())));
    check(
        state.get_books().row_data(0).unwrap().key == "3"
            && !state.get_books_moving()
            && state.get_books_scroll_y() == 0.,
        "reduced motion immediately reveals the new book above a scrolled list",
    );
    theme.set_reduced_motion(false);
    state.set_books_scroll_y(-84.);
    wait(50).await;
    library.session.current = Some(Target::Book(503));
    view.borrow_mut()
        .event(app::Event::Library(Box::new(library.clone())));
    wait(60).await;
    library.session.current = Some(Target::Book(203));
    view.borrow_mut()
        .event(app::Event::Library(Box::new(library)));
    wait(500).await;
    check(
        state.get_books().row_data(0).unwrap().key == "2"
            && state.get_books_scroll_y() == 0.
            && !state.get_books_moving(),
        "rapid changes in a scrolled list settle on the latest book at the top",
    );
    theme.set_reduced_motion(true);
    state.invoke_search("Пикник".into(), 0);
    check(
        state.get_books().row_count() == 1 && state.get_books().row_data(0).unwrap().key == "1",
        "current book still respects the search filter",
    );
    state.invoke_search("".into(), 2);
    check(
        state.get_books().iter().all(|book| book.completed),
        "current book still respects the completed filter",
    );
    state.invoke_search("".into(), 0);
    view.borrow_mut().event(playback(false));
}
