use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Settings {
    pub version: u32,
    pub theme: String,
    pub language: String,
    pub close_to_tray: bool,
    pub playback_rate: f64,
    pub window_width: u32,
    pub window_height: u32,
    pub maximized: bool,
    pub reduced_motion: bool,
    pub text_scale: f32,
    pub android_ui_scale: f32,
    pub skip_silence: bool,
    pub library_sort: String,
    pub library_tab: String,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            theme: "dark".into(),
            language: "system".into(),
            close_to_tray: true,
            playback_rate: 1.0,
            window_width: 1000,
            window_height: 700,
            maximized: false,
            reduced_motion: false,
            text_scale: 1.0,
            android_ui_scale: 1.0,
            skip_silence: false,
            library_sort: "listening".into(),
            library_tab: "all".into(),
        }
    }
}
impl Settings {
    pub fn validate(&mut self) -> Result<()> {
        if self.version != 1 {
            bail!("Unsupported settings version: {}", self.version);
        }
        if !["dark", "light", "system"].contains(&self.theme.as_str()) {
            self.theme = "dark".into();
        }
        if !["ru", "en", "system"].contains(&self.language.as_str()) {
            self.language = "system".into();
        }
        if !["listening", "title", "author", "year", "genre"].contains(&self.library_sort.as_str())
        {
            self.library_sort = "listening".into();
        }
        if !["all", "started"].contains(&self.library_tab.as_str()) {
            self.library_tab = "all".into();
        }
        self.playback_rate = if self.playback_rate.is_finite() {
            self.playback_rate
                .clamp(crate::player::MIN_RATE, crate::player::MAX_RATE)
        } else {
            1.0
        };
        self.window_width = self.window_width.clamp(360, 7680);
        self.window_height = self.window_height.clamp(480, 4320);
        self.text_scale = normalize_scale(self.text_scale, 0.7, 2.05);
        self.android_ui_scale = normalize_scale(self.android_ui_scale, 0.7, 1.6);
        Ok(())
    }
    #[cfg(target_os = "linux")]
    pub fn path() -> PathBuf {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            })
            .join("carlitos/settings.json")
    }
    #[cfg(windows)]
    pub fn path() -> PathBuf {
        crate::platform::data_dir().join("settings.json")
    }
    #[cfg(target_os = "android")]
    pub fn path() -> PathBuf {
        crate::android::context().files.join("settings.json")
    }
    pub fn load(path: &Path) -> (Self, Option<String>, bool) {
        match fs::read(path) {
            Ok(bytes) => match serde_json::from_slice::<Self>(&bytes).and_then(|mut s| {
                s.validate().map_err(serde::de::Error::custom)?;
                Ok(s)
            }) {
                Ok(s) => (s, None, true),
                Err(e) => {
                    // Keep unknown future formats untouched; do not silently downgrade them.
                    let future = serde_json::from_slice::<serde_json::Value>(&bytes)
                        .ok()
                        .and_then(|v| v["version"].as_u64())
                        .is_some_and(|v| v > 1);
                    let backup =
                        path.with_extension(format!("json.recovery-{}", crate::library::now()));
                    let backed_up = !future
                        && fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&backup)
                            .and_then(|mut f| {
                                f.write_all(&bytes)?;
                                f.sync_all()
                            })
                            .is_ok();
                    let note = if backed_up {
                        format!("Settings recovered: {e}. Original: {}", backup.display())
                    } else {
                        format!(
                            "Settings are read-only: {e}. Original preserved at {}",
                            path.display()
                        )
                    };
                    (Self::default(), Some(note), backed_up)
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                (Self::legacy().unwrap_or_default(), None, true)
            }
            Err(e) => (
                Self::default(),
                Some(format!("Cannot read settings: {e}")),
                false,
            ),
        }
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        let mut settings = self.clone();
        settings.validate()?;
        let parent = path.parent().context("Settings directory missing")?;
        let parent = if parent.as_os_str().is_empty() {
            Path::new(".")
        } else {
            parent
        };
        fs::create_dir_all(parent)?;
        // Exclusive creation and RAII cleanup also protect concurrent saves.
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(&serde_json::to_vec_pretty(&settings)?)?;
        temp.write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist(path)?;
        #[cfg(unix)]
        fs::File::open(parent)?.sync_all()?;
        Ok(())
    }
    #[cfg(any(windows, target_os = "android"))]
    fn legacy() -> Option<Self> {
        None
    }
    #[cfg(target_os = "linux")]
    fn legacy() -> Option<Self> {
        let schemas = std::process::Command::new("gsettings")
            .arg("list-schemas")
            .output()
            .ok()?;
        let schema = String::from_utf8_lossy(&schemas.stdout)
            .lines()
            .find(|s| *s == "io.github.mny315.Carlitos")?
            .to_owned();
        let get = |key: &str| -> Option<String> {
            let out = std::process::Command::new("gsettings")
                .args(["get", &schema, key])
                .output()
                .ok()?;
            out.status.success().then(|| {
                String::from_utf8_lossy(&out.stdout)
                    .trim()
                    .trim_matches('\'')
                    .to_owned()
            })
        };
        let mut s = Self::default();
        if let Some(v) = get("theme") {
            s.theme = if v == "black" { "dark".into() } else { v };
        }
        if let Some(v) = get("language") {
            s.language = v;
        }
        if let Some(v) = get("close-to-tray").and_then(|v| v.parse().ok()) {
            s.close_to_tray = v;
        }
        if let Some(v) = get("window-width").and_then(|v| v.parse().ok()) {
            s.window_width = v;
        }
        if let Some(v) = get("window-height").and_then(|v| v.parse().ok()) {
            s.window_height = v;
        }
        if let Some(v) = get("maximized").and_then(|v| v.parse().ok()) {
            s.maximized = v;
        }
        s.validate().ok()?;
        Some(s)
    }
}

fn normalize_scale(value: f32, minimum: f32, maximum: f32) -> f32 {
    if value.is_finite() {
        (value.clamp(minimum, maximum) * 100.0).round() / 100.0
    } else {
        1.0
    }
}

#[cfg(test)]
mod tests;
