#[path = "desktop/instance.rs"]
mod instance;
#[path = "desktop/media.rs"]
mod media;
#[path = "picker.rs"]
pub mod portal;
#[path = "desktop/tray.rs"]
mod tray;

use crate::app::{Command, audio::Playback};
use carlitos::library::Library;
pub use instance::{Connection, Instance};
use media::MediaControls;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    time::Duration,
};
use tray::{WindowState, tray_icon, window_proc};
use windows::{
    Win32::{
        System::LibraryLoader::GetModuleHandleW,
        UI::{
            Shell::{NIM_ADD, NIM_DELETE},
            WindowsAndMessaging::*,
        },
    },
    core::{HSTRING, w},
};

enum Update {
    Library(Box<Library>),
    Playback(Playback),
    Quit,
}
pub struct Desktop {
    tx: Sender<Update>,
    thread: Mutex<Option<std::thread::JoinHandle<()>>>,
    pub tray: Arc<AtomicBool>,
}
impl Desktop {
    pub fn start(instance: Option<(Connection, String)>, commands: Sender<Command>) -> Self {
        let (tx, rx) = mpsc::channel();
        let tray = Arc::new(AtomicBool::new(false));
        let available = tray.clone();
        let startup = commands.clone();
        let thread = std::thread::Builder::new()
            .name("carlitos-desktop".into())
            .spawn(move || {
                let Some((_, name)) = instance else {
                    return;
                };
                let Ok(_apartment) = carlitos::windows::Apartment::new() else {
                    return;
                };
                unsafe {
                    let name = HSTRING::from(name);
                    let module = GetModuleHandleW(None).unwrap_or_default();
                    let class = WNDCLASSW {
                        lpfnWndProc: Some(window_proc),
                        hInstance: module.into(),
                        lpszClassName: windows::core::PCWSTR(name.as_ptr()),
                        ..Default::default()
                    };
                    if RegisterClassW(&class) == 0 {
                        return;
                    }
                    let state = Box::new(WindowState {
                        commands: commands.clone(),
                        tray: available.clone(),
                        taskbar_created: RegisterWindowMessageW(w!("TaskbarCreated")),
                    });
                    let hwnd = match CreateWindowExW(
                        WINDOW_EX_STYLE::default(),
                        &name,
                        w!("Carlitos"),
                        WINDOW_STYLE::default(),
                        0,
                        0,
                        0,
                        0,
                        None,
                        None,
                        Some(module.into()),
                        Some((&*state as *const WindowState).cast()),
                    ) {
                        Ok(hwnd) => hwnd,
                        Err(_) => {
                            let _ = UnregisterClassW(&name, Some(module.into()));
                            return;
                        }
                    };
                    available.store(tray_icon(hwnd, NIM_ADD), Ordering::SeqCst);
                    let media = MediaControls::new(hwnd, commands).ok();
                    loop {
                        let mut msg = MSG::default();
                        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                            let _ = TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                        match rx.recv_timeout(Duration::from_millis(30)) {
                            Ok(Update::Library(library)) => {
                                if let Some(media) = &media {
                                    let _ = media.library(&library);
                                }
                            }
                            Ok(Update::Playback(playback)) => {
                                if let Some(media) = &media {
                                    let _ = media.playback(&playback);
                                }
                            }
                            Ok(Update::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                            Err(mpsc::RecvTimeoutError::Timeout) => {}
                        }
                    }
                    drop(media);
                    tray_icon(hwnd, NIM_DELETE);
                    available.store(false, Ordering::SeqCst);
                    let _ = DestroyWindow(hwnd);
                    let _ = UnregisterClassW(&name, Some(module.into()));
                }
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                let _ = startup.send(Command::Error(format!(
                    "{}: {error}",
                    crate::app::text(
                        "Не удалось запустить интеграцию с рабочим столом",
                        "Could not start desktop integration",
                    )
                )));
                None
            }
        };
        Self {
            tx,
            thread: Mutex::new(thread),
            tray,
        }
    }
    pub fn library(&self, library: &Library) {
        let _ = self.tx.send(Update::Library(Box::new(library.clone())));
    }
    pub fn playback(&self, playback: &Playback) {
        let _ = self.tx.send(Update::Playback(playback.clone()));
    }
    pub fn stop(&self) {
        let _ = self.tx.send(Update::Quit);
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.stop();
    }
}
