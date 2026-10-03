mod books;
mod exercise;
mod imports;
mod parts;
mod scenarios;
mod self_test;
mod sliders;

pub use exercise::exercise;
pub use scenarios::{library_tab, portal, system_theme};
pub use self_test::start;

use crate::{AppWindow, State};
use slint::{
    ComponentHandle,
    platform::{PointerEventButton, WindowEvent},
};
use std::time::Duration;

fn key(window: &AppWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}
fn check(condition: bool, message: &str) {
    if !condition {
        eprintln!("UI CHECK FAILED: {message}");
        std::process::exit(1);
    }
    println!("UI CHECK: {message}");
}
fn click(window: &AppWindow, x: f32, y: f32) {
    let position = slint::LogicalPosition::new(x, y);
    window.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    window
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Left,
        });
}

fn first_library_menu(window: &AppWindow) {
    let state = window.global::<State>();
    let size = window
        .window()
        .size()
        .to_logical(window.window().scale_factor());
    click(
        window,
        size.width - if state.get_compact() { 44. } else { 116. },
        if state.get_compact() { 244. } else { 306. },
    );
}

fn click_book_menu(window: &AppWindow, action: &str) {
    let state = window.global::<State>();
    let size = window
        .window()
        .size()
        .to_logical(window.window().scale_factor());
    let started = state.get_menu_book().started;
    let height = if started { 185. } else { 143. };
    let top = if state.get_book_menu_y() + 46. + height <= size.height - 8. {
        state.get_book_menu_y() + 46.
    } else {
        (state.get_book_menu_y() - height - 8.).max(8.)
    };
    let offset = match action {
        "edit" => 27.,
        "reset" => 111.,
        "remove" => {
            if started {
                158.
            } else {
                116.
            }
        }
        _ => panic!("unknown menu action"),
    };
    click(
        window,
        (state.get_book_menu_x() - 240.).clamp(8., size.width - 248.) + 120.,
        top + offset,
    );
}

fn snapshot(window: &AppWindow, name: &str) {
    let pixels = window.window().take_snapshot().expect("UI snapshot");
    let layout = if window.global::<State>().get_compact() {
        "narrow"
    } else {
        "wide"
    };
    image::save_buffer(
        format!("target/tests/visual/{name}-{layout}.png"),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )
    .expect("save UI snapshot");
}

fn motion_snapshot(window: &AppWindow, name: &'static str) {
    let weak = window.as_weak();
    slint::Timer::single_shot(Duration::from_millis(60), move || {
        if let Some(window) = weak.upgrade() {
            snapshot(&window, name);
        }
    });
}
