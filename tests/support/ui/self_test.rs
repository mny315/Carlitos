mod controls;
mod library;

use super::check;
use crate::{AppWindow, PartItem, State};
use slint::ComponentHandle;
use std::{rc::Rc, time::Duration};

pub fn start(window: &AppWindow, view: &Rc<std::cell::RefCell<crate::app::view::View>>) {
    std::fs::create_dir_all("target/tests/visual").expect("fixture directory");
    let path = std::path::Path::new("target/tests/visual/cover-fixture.png");
    image::RgbImage::from_pixel(64, 96, image::Rgb([65, 89, 113]))
        .save(path)
        .expect("cover fixture");
    check_cover(
        Rc::downgrade(view),
        path.canonicalize()
            .expect("fixture path")
            .display()
            .to_string(),
        std::time::Instant::now(),
    );
    step(
        window.as_weak(),
        window.global::<State>().get_parts(),
        Rc::downgrade(view),
        0,
    );
}
fn check_cover(
    weak: std::rc::Weak<std::cell::RefCell<crate::app::view::View>>,
    path: String,
    started: std::time::Instant,
) {
    slint::Timer::single_shot(Duration::from_millis(100), move || {
        let Some(view) = weak.upgrade() else {
            return;
        };
        // Write the fixture once, and let queued decode results reach the UI
        // even if initial layout delayed several timers until the same frame.
        let ready = view.borrow_mut().test_cover(path.clone());
        if ready || started.elapsed() > Duration::from_secs(5) {
            check(ready, "cover decoding completes on the background worker");
        } else {
            check_cover(weak, path, started);
        }
    });
}

fn step(
    weak: slint::Weak<AppWindow>,
    parts: slint::ModelRc<PartItem>,
    view: std::rc::Weak<std::cell::RefCell<crate::app::view::View>>,
    index: usize,
) {
    slint::Timer::single_shot(
        Duration::from_millis(if index == 0 { 1000 } else { 300 }),
        move || {
            let Some(w) = weak.upgrade() else { return };
            match index {
                0..=13 => controls::step(w, &parts, &view, index),
                14..=32 => library::step(w, &view, index - 14),
                _ => return,
            }
            if index < 32 {
                step(weak, parts, view, index + 1);
            }
        },
    );
}
