use super::{check, key};
use crate::{AppWindow, State};
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{rc::Rc, time::Duration};

pub(super) fn slider_checks(window: &AppWindow) {
    // Observe the callbacks, not just the thumb's optimistic position. In
    // particular a drag must reach the consumer before the button is released.
    let values = Rc::new(std::cell::RefCell::new(Vec::new()));
    let recorded = values.clone();
    window
        .global::<State>()
        .on_set_volume(move |value| recorded.borrow_mut().push(value));
    let recorded = values.clone();
    window
        .global::<State>()
        .on_seek(move |value| recorded.borrow_mut().push(value));
    slider_step(window.as_weak(), values, 0);
}

fn slider_step(
    weak: slint::Weak<AppWindow>,
    values: Rc<std::cell::RefCell<Vec<f32>>>,
    stage: usize,
) {
    slint::Timer::single_shot(Duration::from_millis(100), move || {
        let Some(w) = weak.upgrade() else {
            return;
        };
        let s = w.global::<State>();
        let size = w.window().size().to_logical(w.window().scale_factor());
        let compact = s.get_compact();
        let volume_width = if compact { 45. } else { 68. };
        let volume_y = size.height - if compact { 157. } else { 118. };
        let volume = |fraction: f32| {
            slint::LogicalPosition::new(size.width - 12. - volume_width * (1. - fraction), volume_y)
        };
        let seek = |fraction: f32| {
            slint::LogicalPosition::new(70. + (size.width - 194.) * fraction, size.height - 73.)
        };
        let scroll = |position, delta_y| {
            w.window().dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.,
                delta_y,
            })
        };
        let press = |position| {
            w.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            })
        };
        let release = |position| {
            w.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            })
        };
        let moved = |position| {
            w.window()
                .dispatch_event(WindowEvent::PointerMoved { position })
        };
        let last_is = |expected: f32| {
            values
                .borrow()
                .last()
                .is_some_and(|v| (*v - expected).abs() < 0.02)
        };
        match stage {
            0 => {
                s.set_volume(0.4);
                scroll(volume(0.5), 60.);
                scroll(volume(0.5), 60.);
                check(last_is(0.5), "volume wheel steps accumulate immediately");
                press(volume(0.25));
                check(last_is(0.25), "volume responds on mouse down");
                moved(volume(0.75));
            }
            1 => {
                check(last_is(0.75), "volume changes while mouse remains held");
                release(volume(1.));
                check(last_is(1.), "volume release flushes the final value");
                scroll(volume(0.5), 600.);
                check(s.get_volume() == 1., "volume wheel clamps to maximum");
                s.set_seekable(true);
                s.set_duration_ms(120_000.);
                s.set_position(0.25);
                scroll(seek(0.5), 60.);
                check(
                    last_is(0.25 + 5. / 120.),
                    "seek wheel advances five seconds",
                );
                scroll(seek(0.5), -60.);
                check(last_is(0.25), "seek wheel reverses direction");
                press(seek(0.2));
                check(last_is(0.2), "seek responds on mouse down");
                moved(seek(0.8));
            }
            2 => {
                check(last_is(0.8), "seek changes while mouse remains held");
                release(seek(0.9));
                check(last_is(0.9), "seek release flushes the final position");
                key(&w, Key::Home);
                check(last_is(0.), "slider keyboard navigation is preserved");
                let count = values.borrow().len();
                s.set_seekable(false);
                scroll(seek(0.5), 60.);
                press(seek(0.5));
                release(seek(0.5));
                check(
                    values.borrow().len() == count,
                    "disabled seek rejects wheel and pointer input",
                );
                s.set_seekable(true);
                press(seek(0.2));
                moved(seek(0.8));
                s.set_seekable(false);
                values.borrow_mut().clear();
            }
            3 => {
                // A part change can disable the seek bar during a drag and
                // enable it again before another pointer event arrives.
                s.set_position(0.3);
                s.set_seekable(true);
            }
            4 => {
                release(seek(0.8));
                check(
                    values.borrow().is_empty(),
                    "disabling a seek cancels its pending drag across part changes",
                );
                scroll(seek(0.5), 60.);
                check(
                    last_is(0.3 + 5. / 120.),
                    "seek uses the new part position after a cancelled drag",
                );
                println!("UI CHECKS PASSED");
                s.invoke_action("quit".into(), "".into());
                return;
            }
            _ => return,
        }
        slider_step(weak, values, stage + 1);
    });
}
