#[macro_use]
pub mod i18n;
#[cfg(target_os = "android")]
mod android;
pub mod app;
#[cfg(any(windows, test))]
mod audio_processing;
pub mod import;
pub mod library;
mod platform;
pub mod player;
pub mod settings;
#[cfg(target_os = "linux")]
mod silence;
pub mod source;
pub mod storage;
#[cfg(windows)]
pub mod windows;

slint::include_modules!();
