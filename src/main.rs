#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
pub use carlitos::{
    AppWindow, BookItem, ImportItem, Palette, PartItem, SourceItem, State, Theme, app,
};
mod cli;
#[cfg_attr(windows, path = "windows/desktop.rs")]
mod desktop;
use cli::validate_arguments;
#[cfg(windows)]
#[path = "windows/rendering.rs"]
mod rendering;
#[cfg(all(debug_assertions, target_os = "linux"))]
#[path = "../tests/support/mod.rs"]
mod test_support;
use slint::ComponentHandle;
use slint::winit_030::WinitWindowAccessor;
fn main() -> anyhow::Result<()> {
    let result = run();
    #[cfg(windows)]
    if let Err(error) = &result {
        use windows::{
            Win32::UI::WindowsAndMessaging::{MB_ICONERROR, MB_OK, MessageBoxW},
            core::{HSTRING, w},
        };
        unsafe {
            MessageBoxW(
                None,
                &HSTRING::from(format!("{error:#}")),
                w!("Carlitos"),
                MB_OK | MB_ICONERROR,
            );
        }
    }
    result
}
fn run() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    #[cfg(windows)]
    if args.len() == 2 && args[0] == "--export-licenses" {
        std::fs::write(
            &args[1],
            include_str!(concat!(env!("OUT_DIR"), "/licenses.txt")),
        )?;
        return Ok(());
    }
    validate_arguments(&args)?;
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if args.iter().any(|a| a == "--mock-tray-host") {
        return test_support::desktop::run();
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if args.iter().any(|a| a == "--mock-portal") {
        return test_support::desktop::portal();
    }
    if args.iter().any(|a| a == "--help") {
        println!(
            "Carlitos\n  --demo             isolated design preview\n  --data-dir PATH    use a separate library\n  --snapshot PATH    save a window screenshot\n  --quit-after SEC   exit after a delay\n  --theme dark|light --language ru|en --page 0..5"
        );
        return Ok(());
    }
    let option = |key: &str| {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .cloned()
    };
    let demo = args
        .iter()
        .any(|a| a == "--demo" || a == "--self-test" || a == "--test-cover-theme");
    let data_dir = option("--data-dir")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(carlitos::library::data_dir);
    let instance = if demo {
        None
    } else {
        match desktop::Instance::acquire(&data_dir, !args.iter().any(|a| a == "--no-desktop"))? {
            Some(instance) => Some(instance),
            None => return Ok(()),
        }
    };
    let settings_path = if option("--data-dir").is_some() {
        data_dir.join("settings.json")
    } else {
        carlitos::settings::Settings::path()
    };
    let (mut settings, warning, settings_writable) = if demo {
        (carlitos::settings::Settings::default(), None, false)
    } else {
        carlitos::settings::Settings::load(&settings_path)
    };
    if let Some(theme) = option("--theme") {
        settings.theme = theme;
    }
    if let Some(language) = option("--language") {
        settings.language = language;
    }
    if let Some(size) = option("--size")
        && let Some((width, height)) = size.split_once('x')
    {
        settings.window_width = width.parse()?;
        settings.window_height = height.parse()?;
    }
    settings.validate()?;
    carlitos::i18n::configure(&settings.language);
    #[cfg(windows)]
    rendering::configure()?;
    #[cfg(target_os = "linux")]
    slint::BackendSelector::new().select()?;
    #[cfg(target_os = "linux")]
    slint::set_xdg_app_id("io.github.mny315.Carlitos")?;
    let ui = app::Ui::new(app::Options {
        database: data_dir.join("library.sqlite3"),
        settings_path,
        settings,
        settings_writable,
        demo,
        fake_audio: args.iter().any(|a| a == "--fake-audio"),
    })?;
    let desktop = std::rc::Rc::new(desktop::Desktop::start(
        instance.as_ref().and_then(|i| {
            i.connection
                .clone()
                .map(|connection| (connection, i.activation_name.clone()))
        }),
        ui.app.tx.clone(),
    ));
    let desktop_events = desktop.clone();
    ui.connect(move |event| match event {
        app::Event::Library(l) => desktop_events.library(l),
        app::Event::Playback(p) => desktop_events.playback(p),
        #[cfg(target_os = "linux")]
        app::Event::Volume(volume, muted) => desktop_events.volume(*volume, *muted),
        _ => {}
    })?;
    let app::Ui {
        window,
        mut app,
        view,
        ..
    } = ui;
    let theme_view = std::rc::Rc::downgrade(&view);
    let resize_timer = slint::Timer::default();
    window.window().on_winit_window_event(move |_, event| {
        if matches!(
            event,
            slint::winit_030::winit::event::WindowEvent::Resized(_)
        ) {
            let weak = theme_view.clone();
            resize_timer.start(
                slint::TimerMode::SingleShot,
                std::time::Duration::from_millis(500),
                move || {
                    if let Some(v) = weak.upgrade() {
                        v.borrow_mut().save_window_size();
                    }
                },
            );
        }
        if matches!(
            event,
            slint::winit_030::winit::event::WindowEvent::ThemeChanged(_)
        ) && let Some(v) = theme_view.upgrade()
        {
            v.borrow().system_theme_changed();
        }
        slint::winit_030::EventResult::Propagate
    });
    desktop::portal::bind(&window);
    desktop::portal::watch_theme(&view);
    if let Some(warning) = warning {
        window.global::<State>().set_notice(warning.into());
    }
    let weak_view = std::rc::Rc::downgrade(&view);
    let tray = desktop.tray.clone();
    window.window().on_close_requested(move || {
        if let Some(v) = weak_view.upgrade() {
            v.borrow_mut().save_window_size();
            v.borrow()
                .close(tray.load(std::sync::atomic::Ordering::SeqCst));
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    if let Some(page) = option("--page") {
        window.global::<State>().set_page(page.parse()?);
    }
    if let Some(book) = option("--open-book") {
        let weak = window.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_millis(500), move || {
            if let Some(w) = weak.upgrade() {
                w.global::<State>()
                    .invoke_action("open-book".into(), book.into());
            }
        });
    }
    if let Some(path) = option("--snapshot") {
        let weak = window.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_secs(3), move || {
            if let Some(w) = weak.upgrade() {
                match w.window().take_snapshot() {
                    Ok(pixels) => {
                        if let Err(e) = image::save_buffer(
                            &path,
                            pixels.as_bytes(),
                            pixels.width(),
                            pixels.height(),
                            image::ColorType::Rgba8,
                        ) {
                            eprintln!("Screenshot: {e}");
                        }
                    }
                    Err(e) => eprintln!("Screenshot: {e}"),
                }
            }
        });
    }
    if let Some(delay) = option("--quit-after") {
        let tx = app.tx.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(delay.parse()?), move || {
            let _ = tx.send(app::Command::Quit);
        });
    }
    window.show()?;
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if args.iter().any(|a| a == "--test-theme") {
        test_support::ui::system_theme(&window);
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if let Some(tab) = option("--test-library-tab") {
        test_support::ui::library_tab(&window, tab.parse()?);
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if let Some(mode) = option("--test-portal") {
        test_support::ui::portal(&window, mode);
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if let Some(delays) = option("--test-close-after") {
        for delay in delays.split(',') {
            let weak = window.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_secs(delay.parse()?), move || {
                if let Some(w) = weak.upgrade() {
                    w.window()
                        .dispatch_event(slint::platform::WindowEvent::CloseRequested);
                }
            });
        }
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if args.iter().any(|a| a == "--self-test") {
        test_support::ui::start(&window, &view);
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if args.iter().any(|a| a == "--test-cover-theme") {
        test_support::theme::start(&window, &view);
    }
    #[cfg(all(debug_assertions, target_os = "linux"))]
    if let Some(folder) = option("--exercise") {
        test_support::ui::exercise(&window, folder);
    }
    slint::run_event_loop_until_quit()?;
    app.join();
    desktop.stop();
    Ok(())
}
