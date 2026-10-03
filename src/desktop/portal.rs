use crate::{AppWindow, State, app::text};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::rc::Rc;

/// Wayland's Winit theme describes decorations; the portal supplies the desktop preference.
pub fn watch_theme(view: &Rc<std::cell::RefCell<crate::app::view::View>>) {
    use ashpd::desktop::settings::{ColorScheme, Settings};
    use futures_lite::StreamExt;
    let weak = Rc::downgrade(view);
    let _ = slint::spawn_local(async move {
        loop {
            let update = |scheme: ColorScheme| {
                if let Some(view) = weak.upgrade() {
                    view.borrow_mut().set_system_theme(match scheme {
                        ColorScheme::PreferDark => Some(true),
                        ColorScheme::PreferLight => Some(false),
                        _ => None,
                    });
                }
            };
            if let Ok(proxy) = Settings::new().await
                && let Ok(mut changes) = proxy.receive_color_scheme_changed().await
            {
                if let Ok(scheme) = proxy.color_scheme().await {
                    update(scheme);
                }
                while let Some(scheme) = changes.next().await {
                    if weak.upgrade().is_none() {
                        return;
                    }
                    update(scheme);
                }
            }
            if weak.upgrade().is_none() {
                return;
            }
            let (tx, rx) = async_channel::bounded(1);
            slint::Timer::single_shot(std::time::Duration::from_secs(5), move || {
                let _ = tx.try_send(());
            });
            let _ = rx.recv().await;
        }
    });
}

pub fn bind(window: &AppWindow) {
    bind_picker(window, Picker::Import);
    bind_picker(window, Picker::Cover);
    bind_picker(window, Picker::Source);
}

#[derive(Clone, Copy, PartialEq)]
enum Picker {
    Import,
    Cover,
    Source,
}

fn bind_picker(window: &AppWindow, picker: Picker) {
    let cover = picker == Picker::Cover;
    let weak = window.as_weak();
    let choose = move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let state = window.global::<State>();
        if state.get_picker_busy() {
            return;
        }
        state.set_picker_busy(true);
        let owner = crate::app::view::renew_picker_owner(&window);
        let weak = weak.clone();
        let recovery = weak.clone();
        let spawned = slint::spawn_local(async move {
            let result = async {
                let w = weak
                    .upgrade()
                    .ok_or_else(|| anyhow::anyhow!("Window closed"))?;
                let state = w.global::<State>();
                let folder = std::path::PathBuf::from(match picker {
                    Picker::Import => state.get_import_path().to_string(),
                    Picker::Cover => state.get_edit_cover_path().to_string(),
                    Picker::Source => state.get_relocate_path().to_string(),
                });
                let initial_folder = folder.ancestors().find(|p| p.is_absolute() && p.is_dir());
                let native = w.window().winit_window().await?;
                #[cfg(debug_assertions)]
                eprintln!("Portal: native window ready");
                let window_handle = native.window_handle()?.as_raw();
                let display_handle = native.display_handle()?.as_raw();
                let identifier =
                    ashpd::WindowIdentifier::from_raw_handle(&window_handle, Some(&display_handle))
                        .await;
                #[cfg(debug_assertions)]
                eprintln!("Portal: parent exported");
                let request = ashpd::desktop::file_chooser::SelectedFiles::open_file()
                    .title(match picker {
                        Picker::Cover => text("Выбрать обложку", "Choose cover"),
                        Picker::Import => text("Добавить аудиокниги", "Add audiobooks"),
                        Picker::Source => text("Выбрать папку источника", "Choose source folder"),
                    })
                    .directory(!cover)
                    .current_folder::<&std::path::Path>(initial_folder)?
                    .filters(if cover {
                        vec![
                            ashpd::desktop::file_chooser::FileFilter::new(text(
                                "Изображения",
                                "Images",
                            ))
                            .mimetype("image/png")
                            .mimetype("image/jpeg"),
                        ]
                    } else {
                        vec![]
                    })
                    .multiple(false)
                    .modal(true)
                    .identifier(identifier)
                    .send()
                    .await?;
                #[cfg(debug_assertions)]
                eprintln!("Portal: response received");
                let selected = request.response()?;
                Ok::<_, anyhow::Error>(selected.uris().first().and_then(|u| u.to_file_path().ok()))
            }
            .await;
            if let Some(w) = weak.upgrade() {
                w.global::<State>().set_picker_busy(false);
                if cover
                    && (w.global::<State>().get_overlay() != 2
                        || owner != w.global::<State>().get_picker_owner())
                {
                    return;
                }
                if picker == Picker::Source
                    && (w.global::<State>().get_overlay() != 5
                        || owner != w.global::<State>().get_picker_owner()
                        || w.global::<State>().get_source_busy())
                {
                    return;
                }
                match result {
                    Ok(Some(path)) if cover => {
                        w.global::<State>().invoke_action(
                            "preview-cover".into(),
                            path.display().to_string().into(),
                        );
                    }
                    Ok(Some(path)) if picker == Picker::Source => {
                        w.global::<State>()
                            .set_relocate_path(path.display().to_string().into());
                        w.global::<State>().set_source_error("".into());
                    }
                    Ok(Some(path)) => {
                        w.global::<State>().set_page(2);
                        w.global::<State>()
                            .set_import_path(path.display().to_string().into());
                    }
                    Ok(None) => {}
                    Err(e)
                        if matches!(
                            e.downcast_ref::<ashpd::Error>(),
                            Some(ashpd::Error::Response(
                                ashpd::desktop::ResponseError::Cancelled
                            ))
                        ) => {}
                    Err(e) if cover => w.global::<State>().set_edit_cover_error(
                        format!(
                            "{}: {e}",
                            text(
                                "Не удалось открыть выбор изображения",
                                "Could not open image picker"
                            )
                        )
                        .into(),
                    ),
                    Err(e) => {
                        let message = format!(
                            "{}: {e}. {}",
                            text(
                                "Не удалось открыть выбор папки",
                                "Could not open folder picker"
                            ),
                            text(
                                "Введите путь в поле папки",
                                "Enter the path in the folder field"
                            )
                        );
                        if picker == Picker::Source {
                            w.global::<State>().set_source_error(message.into());
                        } else {
                            w.global::<State>().set_notice(message.into());
                        }
                    }
                }
            }
        });
        if let Err(error) = spawned
            && let Some(window) = recovery.upgrade()
        {
            let state = window.global::<State>();
            state.set_picker_busy(false);
            state.set_notice(error.to_string().into());
        }
    };
    match picker {
        Picker::Cover => window.global::<State>().on_choose_cover(choose),
        Picker::Import => window.global::<State>().on_choose_folder(choose),
        Picker::Source => window.global::<State>().on_choose_source_folder(choose),
    }
}
