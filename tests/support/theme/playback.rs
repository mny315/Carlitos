use super::*;

fn player_strip(window: &AppWindow) -> Vec<u8> {
    let pixels = window.window().take_snapshot().expect("player frame");
    let compact = window.global::<State>().get_compact();
    let scaled = |value: f32| (value * window.window().scale_factor()).round() as u32;
    let panel_height = scaled(if compact { 188. } else { 152. });
    let width = scaled(if compact { 200. } else { 240. }) as usize;
    let padding = scaled(12.);
    let mut strip = Vec::new();
    for y in pixels.height() - panel_height + padding..pixels.height() - padding {
        let offset = (y * pixels.width() + padding) as usize * 4;
        strip.extend_from_slice(&pixels.as_bytes()[offset..offset + width * 4]);
    }
    strip
}

pub(super) async fn player_transition_checks(window: &AppWindow) {
    let state = window.global::<State>();
    let theme = window.global::<Theme>();
    let book = |key: &str, title: &str, color: [u8; 4]| crate::BookItem {
        key: key.into(),
        title: title.into(),
        detail: title.into(),
        cover: slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(
            &color.repeat(34 * 44),
            34,
            44,
        )),
        ..Default::default()
    };
    let first = book("player-a", "First story", [180, 65, 40, 255]);
    let second = book("player-b", "Second story", [40, 85, 180, 255]);
    let third = book("player-c", "Latest story", [55, 145, 65, 255]);
    theme.set_reduced_motion(true);
    state.set_active(true);
    state.set_seekable(true);
    state.set_current(first.clone());
    state.set_part_title("First part".into());
    state.set_position(0.2);
    state.set_position_text("1:00".into());
    state.set_duration_text("5:00".into());
    wait(60).await;
    let before = player_strip(window);
    theme.set_reduced_motion(false);
    state.set_current(second.clone());
    state.set_part_title("Second part".into());
    state.set_position(0.8);
    state.set_position_text("8:00".into());
    state.set_duration_text("10:00".into());
    let mut frames = Vec::new();
    for _ in 0..6 {
        wait(20).await;
        frames.push(player_strip(window));
    }
    wait(350).await;
    let after = player_strip(window);
    check(
        before != after,
        "switching books updates the player content",
    );
    check(
        frames
            .iter()
            .filter(|frame| **frame != before && **frame != after)
            .count()
            >= 3,
        "player paints intermediate frames instead of replacing its content instantly",
    );
    check(
        frames.windows(2).filter(|pair| pair[0] != pair[1]).count() >= 3,
        "player content keeps blending across consecutive frames",
    );
    // A crossfade stays between the old and new pixels, never fading through
    // an empty bar. Check only channels that differ meaningfully at the ends.
    let mut channels = 0;
    let mut bounded = 0;
    for ((&old, &new), &mid) in before.iter().zip(&after).zip(&frames[2]) {
        if old.abs_diff(new) > 30 {
            channels += 1;
            bounded += usize::from(
                mid >= old.min(new).saturating_sub(8) && mid <= old.max(new).saturating_add(8),
            );
        }
    }
    check(
        channels > 100 && bounded * 100 / channels >= 98,
        "player crossfade does not flash an empty background",
    );
    snapshot(window, "player-settled").await;

    state.set_current(first.clone());
    wait(60).await;
    state.set_current(second);
    wait(40).await;
    state.set_current(third);
    wait(750).await;
    let settled = player_strip(window);
    theme.set_reduced_motion(true);
    wait(50).await;
    check(
        player_strip(window) == settled,
        "rapid book switches settle on the latest book without a stale frame",
    );
    theme.set_reduced_motion(false);
    state.set_position(0.3);
    state.set_position_text("3:00".into());
    wait(50).await;
    let seeked = player_strip(window);
    check(
        seeked != settled,
        "playback progress updates without waiting for a book transition",
    );
    wait(400).await;
    check(
        player_strip(window) == seeked,
        "ordinary playback updates do not restart the player animation",
    );
    theme.set_reduced_motion(true);
    state.set_current(first);
    wait(50).await;
    let immediate = player_strip(window);
    wait(400).await;
    check(
        immediate != seeked && player_strip(window) == immediate,
        "reduced motion replaces the player content immediately",
    );
}

pub(super) async fn missing_cover_playback_checks(window: &AppWindow) {
    let state = window.global::<State>();
    let theme = window.global::<Theme>();
    let original_book = state.get_current();
    let original_position = state.get_position_text();
    let original_playing = state.get_playing();
    theme.set_reduced_motion(true);
    state.set_playing(false);
    state.set_current(crate::BookItem {
        key: "without-cover".into(),
        title: "Story without artwork".into(),
        initials: "S".into(),
        ..Default::default()
    });
    state.set_position_text("0:00".into());
    wait(60).await;
    theme.set_reduced_motion(false);
    // Let any book transition settle before ordinary playback updates.
    wait(750).await;
    for second in 1..=3 {
        let mut book = state.get_current();
        book.detail = format!("{second}%").into();
        state.set_current(book);
        state.set_position_text(format!("0:0{second}").into());
        wait(60).await;
        let frame = player_strip(window);
        theme.set_reduced_motion(true);
        wait(60).await;
        check(
            frame == player_strip(window),
            "playback updates without artwork do not restart or queue a crossfade",
        );
        theme.set_reduced_motion(false);
        wait(120).await;
    }
    theme.set_reduced_motion(true);
    state.set_current(original_book);
    state.set_position_text(original_position);
    state.set_playing(original_playing);
    wait(60).await;
    theme.set_reduced_motion(false);
}

fn player_controls(window: &AppWindow) -> Vec<u8> {
    let pixels = window.window().take_snapshot().expect("player controls");
    let scale = window.window().scale_factor();
    let width = pixels.width() as f32 / scale;
    let height = pixels.height() as f32 / scale;
    let transport = if window.global::<State>().get_compact() {
        (100., height - 134., 160., 38.)
    } else {
        (width - 410., height - 140., 280., 44.)
    };
    let mut result = Vec::new();
    for (x, y, w, h) in [transport, (width - 46., height - 85., 24., 24.)] {
        let scaled = |n: f32| (n * scale).round() as usize;
        for row in scaled(y)..scaled(y + h) {
            let start = (row * pixels.width() as usize + scaled(x)) * 4;
            result.extend_from_slice(&pixels.as_bytes()[start..start + scaled(w) * 4]);
        }
    }
    result
}

pub(super) async fn player_loading_checks(window: &AppWindow, view: &Rc<RefCell<app::view::View>>) {
    use app::{Event, audio::Playback};
    use carlitos::{library::Target, player::Phase};
    use slint::platform::{PointerEventButton, WindowEvent};
    let state = window.global::<State>();
    window.global::<Theme>().set_reduced_motion(false);
    let mut library = app::demo_library();
    view.borrow_mut()
        .event(Event::Library(Box::new(library.clone())));
    view.borrow_mut().event(Event::Playback(Playback {
        position: library.session.position,
        duration: library.active_file().and_then(|file| file.duration),
        playing: true,
        seekable: true,
        phase: Phase::Ready,
        ..Default::default()
    }));
    wait(750).await;
    let before = player_controls(window);
    library.session.current = Some(Target::Book(203));
    library.session.position = 120_000;
    library
        .media
        .iter_mut()
        .find(|file| file.id == 203)
        .unwrap()
        .duration = Some(600_000);
    view.borrow_mut()
        .event(Event::Library(Box::new(library.clone())));
    check(
        state.get_position_text() == "2:00" && state.get_duration_text() == "10:00",
        "new book immediately shows its own saved position and cached duration",
    );
    check(
        !state.get_seekable(),
        "switching books immediately blocks seeks to the old file",
    );
    view.borrow_mut().event(Event::Playback(Playback {
        position: 120_000,
        playing: true,
        phase: Phase::Loading,
        ..Default::default()
    }));
    for _ in 0..3 {
        wait(100).await;
        check(
            player_controls(window) == before,
            "loading does not dim or flash the clock and skip controls",
        );
        check(
            state.get_duration_text() == "10:00" && state.get_position_text() == "2:00",
            "loading does not replace known timing with placeholders",
        );
    }
    snapshot(window, "player-loading").await;
    let calls = Rc::new(RefCell::new(Vec::new()));
    let actions = calls.clone();
    state.on_action(move |action, _| actions.borrow_mut().push(action.to_string()));
    let seeks = calls.clone();
    state.on_seek(move |_| seeks.borrow_mut().push("seek".into()));
    let size = window
        .window()
        .size()
        .to_logical(window.window().scale_factor());
    let skip = if state.get_compact() {
        (226., size.height - 115.)
    } else {
        (size.width - 210., size.height - 118.)
    };
    for (x, y) in [
        skip,
        (size.width - 34., size.height - 73.),
        (size.width / 2., size.height - 73.),
    ] {
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
    check(
        calls.borrow().is_empty() && state.get_overlay() == 0,
        "loading controls retain their appearance but reject seek and clock clicks",
    );
    view.borrow_mut().event(Event::Playback(Playback {
        position: 120_000,
        duration: Some(600_000),
        playing: true,
        seekable: true,
        phase: Phase::Ready,
        ..Default::default()
    }));
    wait(80).await;
    check(
        player_controls(window) == before,
        "ready playback restores input without relighting the controls",
    );
    view.borrow_mut().event(Event::Playback(Playback {
        position: 120_000,
        duration: Some(600_000),
        phase: Phase::Error,
        ..Default::default()
    }));
    wait(200).await;
    check(
        !state.get_seek_controls_lit() && player_controls(window) != before,
        "an actual playback error still displays disabled controls",
    );

    library.session.current = Some(Target::Book(300));
    library.session.position = 0;
    library
        .media
        .iter_mut()
        .find(|file| file.id == 300)
        .unwrap()
        .duration = None;
    view.borrow_mut().event(Event::Library(Box::new(library)));
    view.borrow_mut().event(Event::Playback(Playback {
        phase: Phase::Loading,
        ..Default::default()
    }));
    check(
        state.get_duration_text() == "—:—" && state.get_position_text() == "0:00",
        "a file of unknown duration never borrows the previous book's time",
    );
}
