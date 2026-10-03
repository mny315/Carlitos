use crate::{AppWindow, State, app::text};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::rc::Rc;

pub fn watch_theme(_: &Rc<std::cell::RefCell<crate::app::view::View>>) {
    // The existing Winit ThemeChanged handler reports Windows' preference.
}
pub fn bind(window: &AppWindow) {
    for picker in [Picker::Import, Picker::Cover, Picker::Source] {
        bind_picker(window, picker);
    }
}
#[derive(Clone, Copy, PartialEq)]
enum Picker {
    Import,
    Cover,
    Source,
}
fn bind_picker(window: &AppWindow, picker: Picker) {
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
            let Some(w) = weak.upgrade() else {
                return;
            };
            let state = w.global::<State>();
            let folder = std::path::PathBuf::from(
                match picker {
                    Picker::Import => state.get_import_path(),
                    Picker::Cover => state.get_edit_cover_path(),
                    Picker::Source => state.get_relocate_path(),
                }
                .as_str(),
            );
            let mut dialog = rfd::AsyncFileDialog::new().set_title(match picker {
                Picker::Import => text("Добавить аудиокниги", "Add audiobooks"),
                Picker::Cover => text("Выбрать обложку", "Choose cover"),
                Picker::Source => text("Выбрать папку источника", "Choose source folder"),
            });
            if let Some(folder) = folder.ancestors().find(|p| p.is_absolute() && p.is_dir()) {
                dialog = dialog.set_directory(folder);
            }
            if let Ok(native) = w.window().winit_window().await {
                dialog = dialog.set_parent(native.as_ref());
            }
            let selected = if picker == Picker::Cover {
                dialog
                    .add_filter(text("Изображения", "Images"), &["jpg", "jpeg", "png"])
                    .pick_file()
                    .await
            } else {
                dialog.pick_folder().await
            };
            let Some(w) = weak.upgrade() else {
                return;
            };
            let state = w.global::<State>();
            state.set_picker_busy(false);
            if picker == Picker::Cover
                && (state.get_overlay() != 2 || state.get_picker_owner() != owner)
            {
                return;
            }
            if picker == Picker::Source
                && (state.get_overlay() != 5
                    || state.get_picker_owner() != owner
                    || state.get_source_busy())
            {
                return;
            }
            if let Some(file) = selected {
                let path = file.path().to_string_lossy().into_owned().into();
                match picker {
                    Picker::Cover => state.invoke_action("preview-cover".into(), path),
                    Picker::Import => {
                        state.set_page(2);
                        state.set_import_path(path);
                    }
                    Picker::Source => {
                        state.set_relocate_path(path);
                        state.set_source_error("".into());
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
