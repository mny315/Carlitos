use super::super::{check, click, key, motion_snapshot};
use crate::{AppWindow, PartItem, State, Theme};
use slint::{
    ComponentHandle, Model, VecModel,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::rc::Rc;

pub(super) fn step(
    w: AppWindow,
    parts: &slint::ModelRc<PartItem>,
    view: &std::rc::Weak<std::cell::RefCell<crate::app::view::View>>,
    index: usize,
) {
    let s = w.global::<State>();
    match index {
        0 => {
            check(
                s.get_books().row_count() == 6,
                "demo library loaded without opening user data",
            );
            w.invoke_focus_search();
            key(&w, " ");
        }
        1 => {
            check(s.get_query().as_str() == " ", "Space stays in text input");
            key(&w, Key::Backspace);
            key(&w, "Солярис");
        }
        2 => {
            check(
                s.get_books().row_count() == 1,
                "Cyrillic search filters the real model",
            );
            s.invoke_action("open-book".into(), "1".into());
        }
        3 => {
            check(
                s.get_page() == 1 && s.get_parts().row_count() == 12,
                "book and contents navigation",
            );
            s.set_page(3);
            s.set_language_index(2);
            s.invoke_settings_changed();
        }
        4 => {
            check(
                w.global::<Theme>().get_english(),
                "language changes without restart",
            );
            let scale = w.window().scale_factor();
            s.set_text_size_percent(123);
            s.set_ui_scale_percent(125);
            s.invoke_display_settings_preview();
            check(
                (w.global::<Theme>().get_font_scale() - 1.23).abs() < 0.001,
                "precise text size applies during preview before saving",
            );
            check(
                w.window().scale_factor() == scale,
                "Android interface scale does not affect desktop density",
            );
            s.set_text_size_percent(100);
            s.set_ui_scale_percent(100);
            s.set_theme_index(1);
            s.invoke_settings_changed();
            s.set_skip_silence(true);
            s.invoke_action("skip-silence".into(), "".into());
        }
        5 => {
            check(
                s.get_skip_silence(),
                "silence skip setting reaches the controller",
            );
            check(
                !w.global::<Theme>().get_dark(),
                "light theme changes without restart",
            );
            w.global::<Theme>().set_reduced_motion(true);
            s.invoke_settings_changed();
            s.invoke_action("begin-edit".into(), "".into());
        }
        6 => {
            check(s.get_overlay() == 2, "in-window edit confirmation");
            key(&w, Key::Escape);
        }
        7 => {
            check(s.get_overlay() == 0, "Escape closes the dialog");
            if let Some(view) = view.upgrade() {
                super::super::imports::draft_checks(&w, &view);
            }
            s.set_page(1);
            s.set_contents(true);
            s.set_parts(
                Rc::new(VecModel::from(
                    (0..10_000)
                        .map(|i| PartItem {
                            key: format!("{i}:0").into(),
                            title: format!("Chapter {} — virtualized long audiobook", i + 1).into(),
                            detail: "23:45".into(),
                            number: (i + 1).to_string().into(),
                            ..Default::default()
                        })
                        .collect::<Vec<_>>(),
                ))
                .into(),
            );
        }
        8 => {
            check(
                s.get_parts().row_count() == 10_000,
                "10,000-part model installed",
            );
            w.window().dispatch_event(WindowEvent::PointerScrolled {
                position: slint::LogicalPosition::new(280., 270.),
                delta_x: 0.,
                delta_y: -80_000.,
            });
        }
        9 => {
            check(
                s.get_parts().row_count() == 10_000,
                "large-list scroll leaves the event loop responsive",
            );
            let pixels = w.window().take_snapshot().expect("UI snapshot");
            std::fs::create_dir_all("target/tests/visual").expect("snapshot directory");
            image::save_buffer(
                "target/tests/visual/stress-10000.png",
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                image::ColorType::Rgba8,
            )
            .expect("save snapshot");
            s.set_parts(parts.clone());
            s.set_page(3);
            w.invoke_focus_shell();
            w.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Control.into(),
            });
            key(&w, "l");
            w.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
        }
        10 => {
            check(s.get_page() == 0, "Ctrl+L returns to the library");
            let size = w.window().size().to_logical(w.window().scale_factor());
            let position = slint::LogicalPosition::new(size.width - 40., size.height - 30.);
            w.window().dispatch_event(WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Left,
            });
            w.window().dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Left,
            });
        }
        11 => {
            check(s.get_overlay() == 6, "speed button opens its controls");
            let pixels = w.window().take_snapshot().expect("speed dialog snapshot");
            image::save_buffer(
                "target/tests/visual/speed-dialog.png",
                pixels.as_bytes(),
                pixels.width(),
                pixels.height(),
                image::ColorType::Rgba8,
            )
            .expect("save speed dialog");
            s.invoke_set_rate(1.5);
        }
        12 => {
            check(
                s.get_rate() == 1.5,
                "speed selection reaches the controller and updates the UI",
            );
            let size = w.window().size().to_logical(w.window().scale_factor());
            let x = size.width / 2. + if s.get_compact() { 118. } else { 180. };
            let y = size.height / 2. - 45.;
            click(&w, x, y);
            click(&w, x, y);
            key(&w, Key::Escape);
        }
        13 => {
            check(s.get_overlay() == 0, "Escape closes speed controls");
            check(
                (s.get_rate() - 1.6).abs() < 0.001,
                "rapid speed increments are not collapsed into one change",
            );
            s.set_language_index(1);
            w.global::<Theme>().set_reduced_motion(false);
            s.invoke_settings_changed();
            s.invoke_action("open-book".into(), "2".into());
            motion_snapshot(&w, "book-transition");
        }
        _ => unreachable!(),
    }
}
