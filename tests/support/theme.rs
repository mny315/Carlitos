#[path = "theme/playback.rs"]
mod playback;
use playback::{missing_cover_playback_checks, player_loading_checks, player_transition_checks};

use crate::{AppWindow, State, Theme, app};
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
use std::{cell::RefCell, rc::Rc, time::Duration};

async fn wait(milliseconds: u64) {
    let (tx, rx) = async_channel::bounded(1);
    slint::Timer::single_shot(Duration::from_millis(milliseconds), move || {
        let _ = tx.try_send(());
    });
    rx.recv().await.expect("theme test timer");
}

fn check(condition: bool, message: &str) {
    if !condition {
        eprintln!("COVER THEME CHECK FAILED: {message}");
        std::process::exit(1);
    }
    println!("COVER THEME CHECK: {message}");
}

async fn snapshot(window: &AppWindow, name: &str) {
    let pixels = window.window().take_snapshot().expect("theme snapshot");
    let layout = if window.global::<State>().get_compact() {
        "narrow"
    } else {
        "wide"
    };
    image::save_buffer(
        format!("target/tests/visual/theme-{name}-{layout}.png"),
        pixels.as_bytes(),
        pixels.width(),
        pixels.height(),
        image::ColorType::Rgba8,
    )
    .expect("save theme snapshot");
}

pub fn start(window: &AppWindow, view: &Rc<RefCell<app::view::View>>) {
    // This scenario supplies its fixture directly to View. Disable geometry
    // persistence so compositor resize events cannot restore the controller's
    // original demo library over the temporary artwork.
    window
        .window()
        .on_winit_window_event(|_, _| slint::winit_030::EventResult::Propagate);
    let weak = window.as_weak();
    let view = view.clone();
    slint::spawn_local(async move {
        wait(1500).await;
        let Some(window) = weak.upgrade() else { return };
        let theme = window.global::<Theme>();
        let state = window.global::<State>();
        let neutral_dark = theme.get_book_accent_dark();
        let neutral_light = theme.get_book_accent_light();
        check(
            neutral_dark.red() == neutral_dark.green()
                && neutral_dark.green() == neutral_dark.blue(),
            "no artwork starts neutral",
        );
        std::fs::create_dir_all("target/tests/visual").unwrap();
        snapshot(&window, "neutral").await;
        missing_cover_playback_checks(&window).await;
        let directory = tempfile::tempdir().unwrap();
        let mut library = app::demo_library();
        for (index, rgb) in [[35, 85, 170], [180, 90, 30], [130, 130, 130]]
            .into_iter()
            .enumerate()
        {
            let path = directory.path().join(format!("{index}.png"));
            image::RgbImage::from_pixel(80, 108, image::Rgb(rgb))
                .save(&path)
                .unwrap();
            library.books[index].cover = Some(path.to_string_lossy().into_owned());
        }
        library.books[3].cover = Some(
            directory
                .path()
                .join("missing.png")
                .to_string_lossy()
                .into_owned(),
        );
        view.borrow_mut()
            .event(app::Event::Library(Box::new(library.clone())));
        for _ in 0..60 {
            if theme.get_book_accent_dark() != neutral_dark {
                break;
            }
            wait(50).await;
        }
        wait(850).await;
        let blue_dark = theme.get_book_accent_dark();
        let blue_light = theme.get_book_accent_light();
        check(
            blue_dark.blue() > blue_dark.red(),
            "restored current book picks up its asynchronously decoded cover",
        );
        check(
            theme.get_accent()
                == if theme.get_dark() {
                    blue_dark
                } else {
                    blue_light
                },
            "animated palette reaches the cover color",
        );
        snapshot(&window, "blue").await;
        state.invoke_action("open-book".into(), "2".into());
        wait(100).await;
        check(
            theme.get_book_accent_dark() == blue_dark,
            "browsing another book keeps the player's color",
        );
        view.borrow_mut()
            .event(app::Event::Playback(app::audio::Playback::default()));
        check(
            theme.get_book_accent_dark() == blue_dark,
            "pausing retains the current book's palette",
        );

        library.session.current = Some(carlitos::library::Target::Book(200));
        let before = theme.get_accent();
        view.borrow_mut()
            .event(app::Event::Library(Box::new(library.clone())));
        let target = if theme.get_dark() {
            theme.get_book_accent_dark()
        } else {
            theme.get_book_accent_light()
        };
        check(target != before, "switching books chooses a different hue");
        let start = theme.get_accent();
        check(
            start == before,
            "color transition starts at the previous palette",
        );
        let mut frames = std::collections::HashSet::new();
        for _ in 0..10 {
            wait(50).await;
            let pixels = window
                .window()
                .take_snapshot()
                .expect("animated palette frame");
            let accent = theme.get_accent();
            let rgb = [accent.red(), accent.green(), accent.blue()];
            frames.insert(rgb);
            let matching = pixels
                .as_bytes()
                .as_chunks::<4>()
                .0
                .iter()
                .filter(|pixel| pixel[..3].iter().zip(rgb).all(|(&a, b)| a.abs_diff(b) <= 2))
                .count();
            check(
                matching > 20,
                "painted accents track the shared transition on each frame",
            );
        }
        check(
            frames.len() >= 6,
            "transition paints many distinct intermediate colors",
        );
        wait(250).await;
        check(theme.get_accent() == target, "color transition finishes");
        check(
            !theme.get_transitioning(),
            "completed transition stops its frame clock",
        );
        snapshot(&window, "amber").await;

        library.session.current = Some(carlitos::library::Target::Book(100));
        view.borrow_mut()
            .event(app::Event::Library(Box::new(library.clone())));
        wait(180).await;
        let midway = theme.get_accent();
        library.session.current = Some(carlitos::library::Target::Book(200));
        view.borrow_mut()
            .event(app::Event::Library(Box::new(library.clone())));
        check(
            theme.get_accent() == midway,
            "rapid book changes continue from the displayed color",
        );
        wait(850).await;
        theme.set_dark(!theme.get_dark());
        check(
            theme.get_accent()
                == if theme.get_dark() {
                    theme.get_book_accent_dark()
                } else {
                    theme.get_book_accent_light()
                },
            "switching light/dark preserves the hue with immediate readable contrast",
        );
        wait(100).await;
        snapshot(&window, "amber-alternate").await;

        theme.set_reduced_motion(true);
        for part in [300, 400, 500] {
            library.session.current = Some(carlitos::library::Target::Book(part));
            view.borrow_mut()
                .event(app::Event::Library(Box::new(library.clone())));
            wait(30).await;
            check(
                theme.get_book_accent_dark() == neutral_dark
                    && theme.get_book_accent_light() == neutral_light,
                "grayscale, unreadable and absent covers fall back to neutral",
            );
            check(
                theme.get_accent()
                    == if theme.get_dark() {
                        neutral_dark
                    } else {
                        neutral_light
                    },
                "reduced motion applies the palette immediately",
            );
        }
        library.session.current = Some(carlitos::library::Target::Book(100));
        library.books[0].cover = library.books[1].cover.clone();
        view.borrow_mut()
            .event(app::Event::Library(Box::new(library.clone())));
        check(
            theme.get_book_accent_dark().red() > theme.get_book_accent_dark().blue(),
            "replacing a current book's cover updates its palette",
        );
        library.session.current = None;
        view.borrow_mut()
            .event(app::Event::Library(Box::new(library)));
        check(
            theme.get_book_accent_dark() == neutral_dark,
            "removing the current book clears the accent",
        );
        player_transition_checks(&window).await;
        player_loading_checks(&window, &view).await;
        println!("COVER THEME CHECKS PASSED");
        view.borrow().send(app::Command::Quit);
    })
    .expect("start cover theme checks");
}
