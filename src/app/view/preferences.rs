use super::View;
use crate::{State, Theme, app::Command};
use slint::ComponentHandle;
#[cfg(any(target_os = "linux", windows))]
use slint::winit_030::WinitWindowAccessor;

impl View {
    pub(super) fn apply_settings(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let s = w.global::<State>();
        let theme = w.global::<Theme>();
        crate::i18n::configure(&self.settings.language);
        let language_changed = theme.get_english() != crate::i18n::english();
        theme.set_english(crate::i18n::english());
        if language_changed {
            self.refresh_drafts();
        }
        let dark = match self.settings.theme.as_str() {
            "light" => false,
            "system" => self.system_dark.unwrap_or_else(|| {
                #[cfg(target_os = "android")]
                {
                    crate::android::context().configuration.lock().unwrap().dark
                }
                #[cfg(any(target_os = "linux", windows))]
                w.window()
                    .with_winit_window(|native| {
                        native.theme() != Some(slint::winit_030::winit::window::Theme::Light)
                    })
                    .unwrap_or(true)
            }),
            _ => true,
        };
        theme.set_dark(dark);
        #[cfg(target_os = "android")]
        let _ =
            crate::android::bridge::dispatch(serde_json::json!({"op":"system-bars", "dark":dark}));
        theme.set_reduced_motion(self.settings.reduced_motion);
        self.apply_display_settings();
        w.invoke_sync_theme();
        s.set_theme_index(match self.settings.theme.as_str() {
            "light" => 1,
            "system" => 2,
            _ => 0,
        });
        s.set_language_index(match self.settings.language.as_str() {
            "ru" => 1,
            "en" => 2,
            _ => 0,
        });
        s.set_close_to_tray(self.settings.close_to_tray);
        s.set_rate(self.settings.playback_rate as f32);
        s.set_skip_silence(self.settings.skip_silence);
        let sort_index = ["listening", "title", "author", "year", "genre"]
            .iter()
            .position(|s| *s == self.settings.library_sort)
            .unwrap_or(0) as i32;
        s.set_sort_index(sort_index);
        s.set_completed_sort_index((sort_index - 1).max(0));
    }
    fn apply_display_settings(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let s = w.global::<State>();
        let theme = w.global::<Theme>();
        s.set_text_size_percent((self.settings.text_scale * 100.0).round() as i32);
        s.set_ui_scale_percent((self.settings.android_ui_scale * 100.0).round() as i32);
        #[cfg(any(target_os = "linux", windows))]
        theme.set_font_scale(self.settings.text_scale);
        #[cfg(target_os = "android")]
        {
            let system_font = crate::android::context()
                .configuration
                .lock()
                .unwrap()
                .font_scale;
            theme.set_font_scale(system_font * self.settings.text_scale);
            if let Err(error) =
                i_slint_backend_android_activity::set_ui_scale(self.settings.android_ui_scale)
            {
                eprintln!("Android interface scale: {error}");
            }
        }
    }
    pub(super) fn preview_display_settings(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let s = w.global::<State>();
        self.settings.text_scale = s.get_text_size_percent().clamp(70, 205) as f32 / 100.0;
        self.settings.android_ui_scale = s.get_ui_scale_percent().clamp(70, 160) as f32 / 100.0;
        self.apply_display_settings();
    }
    pub(super) fn save_settings(&mut self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let s = w.global::<State>();
        let t = w.global::<Theme>();
        self.settings.theme = ["dark", "light", "system"]
            .get(s.get_theme_index() as usize)
            .unwrap_or(&"dark")
            .to_string();
        self.settings.language = ["system", "ru", "en"]
            .get(s.get_language_index() as usize)
            .unwrap_or(&"system")
            .to_string();
        self.settings.close_to_tray = s.get_close_to_tray();
        self.settings.reduced_motion = t.get_reduced_motion();
        self.settings.text_scale = s.get_text_size_percent().clamp(70, 205) as f32 / 100.0;
        self.settings.android_ui_scale = s.get_ui_scale_percent().clamp(70, 160) as f32 / 100.0;
        self.settings.library_sort = ["listening", "title", "author", "year", "genre"]
            .get(s.get_sort_index() as usize)
            .unwrap_or(&"listening")
            .to_string();
        self.apply_settings();
        self.refresh_books();
        self.send(Command::Settings(self.settings.clone()));
    }
    pub fn save_window_size(&mut self) {
        if let Some(w) = self.window.upgrade() {
            if self.hidden {
                return;
            }
            let size = w.window().size().to_logical(w.window().scale_factor());
            self.settings.maximized = w.window().is_maximized();
            if !self.settings.maximized {
                self.settings.window_width = size.width as u32;
                self.settings.window_height = size.height as u32;
            }
            self.send(Command::Settings(self.settings.clone()));
        }
    }
    pub fn system_theme_changed(&self) {
        if self.settings.theme == "system" {
            self.apply_settings();
        }
    }
    #[cfg(target_os = "android")]
    pub fn android_configuration_changed(&mut self) {
        self.apply_settings();
        self.refresh_books();
        self.refresh_selected();
        self.refresh_current();
        self.refresh_drafts();
    }
    #[cfg(target_os = "linux")]
    pub fn set_system_theme(&mut self, dark: Option<bool>) {
        self.system_dark = dark;
        self.system_theme_changed();
    }
}
