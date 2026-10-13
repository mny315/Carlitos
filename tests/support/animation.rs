//! Measure real animation frames without requesting extra redraws or taking screenshots.
use crate::{AppWindow, BookItem, State, Theme};
use serde_json::{Value, json};
use slint::{ComponentHandle, ModelRc, RenderingState, VecModel};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Frames {
    previous: Option<Instant>,
    intervals: Vec<f64>,
}

async fn wait(ms: u64) {
    let (tx, rx) = async_channel::bounded(1);
    slint::Timer::single_shot(Duration::from_millis(ms), move || {
        let _ = tx.try_send(());
    });
    rx.recv().await.expect("animation measurement timer");
}

pub fn start(window: &AppWindow, finished: impl FnOnce(Value) + 'static) {
    let frames = Rc::new(RefCell::new(Frames::default()));
    let recording = frames.clone();
    let weak = window.as_weak();
    window
        .window()
        .set_rendering_notifier(move |phase, _| {
            if !matches!(phase, RenderingState::AfterRendering) {
                return;
            }
            let mut frames = recording.borrow_mut();
            if weak
                .upgrade()
                .is_some_and(|w| w.window().has_active_animations())
            {
                let now = Instant::now();
                if let Some(previous) = frames.previous {
                    frames
                        .intervals
                        .push(now.duration_since(previous).as_secs_f64() * 1000.);
                }
                frames.previous = Some(now);
            } else {
                frames.previous = None;
            }
        })
        .expect("renderer frame measurement");
    let weak = window.as_weak();
    slint::spawn_local(async move {
        wait(800).await;
        let w = weak.upgrade().expect("measurement window");
        let s = w.global::<State>();
        let t = w.global::<Theme>();
        t.set_reduced_motion(false);
        t.set_english(true);
        s.set_loading(false);
        let books = vec![
            BookItem {
                key: "fps-a".into(),
                title: "Animation A".into(),
                initials: "A".into(),
                started: true,
                ..Default::default()
            },
            BookItem {
                key: "fps-b".into(),
                title: "Animation B".into(),
                initials: "B".into(),
                started: true,
                ..Default::default()
            },
        ];
        let mut report = Vec::new();
        for scene in [
            "pages",
            "dialog",
            "book-menu",
            "notice",
            "palette",
            "player",
            "book-move",
            "playing-indicator",
            "busy-indicator",
            "sync-indicator",
        ] {
            s.set_page(0);
            s.set_overlay(0);
            s.set_book_menu(false);
            s.set_notice("".into());
            s.set_scanning(false);
            s.set_playing(false);
            s.set_sync_indexing(false);
            s.set_books(ModelRc::new(VecModel::from(books.clone())));
            s.set_current(books[0].clone());
            s.set_active(true);
            wait(800).await;
            *frames.borrow_mut() = Frames::default();
            for step in 0..6 {
                let on = step % 2 == 0;
                match scene {
                    "pages" => s.set_page(if on { 3 } else { 0 }),
                    "dialog" => s.set_overlay(if on { 6 } else { 0 }),
                    "book-menu" => {
                        s.set_menu_book(books[0].clone());
                        s.set_book_menu_x(600.);
                        s.set_book_menu_y(250.);
                        s.set_book_menu(on);
                    }
                    "notice" => s.set_notice(if on { "Animation measurement" } else { "" }.into()),
                    "palette" => {
                        let c = if on {
                            slint::Color::from_rgb_u8(120, 170, 200)
                        } else {
                            slint::Color::from_rgb_u8(220, 130, 90)
                        };
                        t.invoke_set_book_colors(c, c);
                    }
                    "player" => s.set_current(books[usize::from(on)].clone()),
                    "book-move" => {
                        let mut moving = books.clone();
                        moving[0].move_offset = if on { 1. } else { -1. };
                        s.set_books(ModelRc::new(VecModel::from(moving)));
                        s.invoke_start_book_move();
                    }
                    "playing-indicator" => s.set_playing(true),
                    "busy-indicator" => {
                        s.set_page(2);
                        s.set_scan_status("Scanning…".into());
                        s.set_scanning(true);
                    }
                    "sync-indicator" => {
                        s.set_local_sync(true);
                        s.set_sync_indexing(true);
                    }
                    _ => unreachable!(),
                }
                wait(if scene == "palette" { 680 } else { 450 }).await;
            }
            let mut samples = frames.borrow().intervals.clone();
            samples.sort_by(f64::total_cmp);
            let count = samples.len();
            let percentile = |p: f64| {
                samples
                    .get(((count.saturating_sub(1)) as f64 * p).round() as usize)
                    .copied()
                    .unwrap_or(0.)
            };
            let median = percentile(0.5);
            report.push(
                json!({"scene":scene,"intervals":count,"median_ms":median,"p95_ms":percentile(0.95),
                "median_fps": if median > 0. { 1000. / median } else { 0. },
                "over_20ms":samples.iter().filter(|v| **v > 20.).count()}),
            );
        }
        s.set_scanning(false);
        s.set_playing(false);
        s.set_sync_indexing(false);
        finished(json!(report));
    })
    .expect("animation measurement task");
}
