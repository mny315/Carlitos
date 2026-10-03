use super::tray::ACTIVATE;
use std::{
    fs::{File, OpenOptions},
    hash::{Hash, Hasher},
};
use windows::{
    Win32::{
        Foundation::{LPARAM, WPARAM},
        UI::WindowsAndMessaging::{
            AllowSetForegroundWindow, FindWindowW, GetWindowThreadProcessId, PostMessageW,
        },
    },
    core::HSTRING,
};

#[derive(Clone)]
pub struct Connection;
pub struct Instance {
    pub connection: Option<Connection>,
    pub activation_name: String,
    _lock: File,
}
impl Instance {
    pub fn acquire(data: &std::path::Path, desktop: bool) -> anyhow::Result<Option<Self>> {
        std::fs::create_dir_all(data)?;
        let mut hash = std::collections::hash_map::DefaultHasher::new();
        // Windows paths are case insensitive; directory aliases are canonicalized.
        data.canonicalize()?
            .to_string_lossy()
            .to_lowercase()
            .hash(&mut hash);
        let activation_name = format!("Carlitos.Library.{:016x}", hash.finish());
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(data.join("instance.lock"))?;
        if let Err(error) = lock.try_lock() {
            if let std::fs::TryLockError::Error(error) = error {
                return Err(error.into());
            }
            if desktop {
                unsafe {
                    if let Ok(hwnd) = FindWindowW(&HSTRING::from(&activation_name), None) {
                        let mut pid = 0;
                        GetWindowThreadProcessId(hwnd, Some(&mut pid));
                        let _ = AllowSetForegroundWindow(pid);
                        let _ = PostMessageW(Some(hwnd), ACTIVATE, WPARAM(0), LPARAM(0));
                    }
                }
            }
            return Ok(None);
        }
        Ok(Some(Self {
            connection: desktop.then_some(Connection),
            activation_name,
            _lock: lock,
        }))
    }
}
