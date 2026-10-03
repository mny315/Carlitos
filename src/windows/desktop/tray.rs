use crate::app::Command;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Sender,
};
use windows::{
    Win32::{
        Foundation::*,
        System::LibraryLoader::GetModuleHandleW,
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::HSTRING,
};

pub(super) const ACTIVATE: u32 = WM_APP + 1;
const TRAY: u32 = WM_APP + 2;

pub(super) struct WindowState {
    pub(super) commands: Sender<Command>,
    pub(super) tray: Arc<AtomicBool>,
    pub(super) taskbar_created: u32,
}
pub(super) unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
) -> LRESULT {
    unsafe {
        if msg == WM_NCCREATE {
            let create = &*(lp.0 as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const WindowState;
        if !ptr.is_null() {
            let state = &*ptr;
            let command = if msg == ACTIVATE || (msg == TRAY && lp.0 as u32 == WM_LBUTTONUP) {
                Some(Command::Show)
            } else if msg == TRAY && lp.0 as u32 == WM_RBUTTONUP {
                if let Ok(menu) = CreatePopupMenu() {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        1,
                        &HSTRING::from(crate::app::text("Показать Carlitos", "Show Carlitos")),
                    );
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        2,
                        &HSTRING::from(crate::app::text("Воспроизведение / пауза", "Play / pause")),
                    );
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        3,
                        &HSTRING::from(crate::app::text("Выйти", "Quit")),
                    );
                    let mut point = POINT::default();
                    let _ = GetCursorPos(&mut point);
                    let _ = SetForegroundWindow(hwnd);
                    let selected = TrackPopupMenu(
                        menu,
                        TPM_RETURNCMD | TPM_RIGHTBUTTON,
                        point.x,
                        point.y,
                        Some(0),
                        hwnd,
                        None,
                    )
                    .0;
                    let _ = DestroyMenu(menu);
                    match selected {
                        1 => Some(Command::Show),
                        2 => Some(Command::Toggle),
                        3 => Some(Command::Quit),
                        _ => None,
                    }
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(command) = command {
                let _ = state.commands.send(command);
                return LRESULT(0);
            }
            if msg == state.taskbar_created {
                let available = tray_icon(hwnd, NIM_ADD);
                state.tray.store(available, Ordering::SeqCst);
                if !available {
                    let _ = state.commands.send(Command::Show);
                }
            }
        }
        DefWindowProcW(hwnd, msg, wp, lp)
    }
}
pub(super) unsafe fn tray_icon(hwnd: HWND, action: NOTIFY_ICON_MESSAGE) -> bool {
    unsafe {
        let mut icon = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_MESSAGE | NIF_ICON | NIF_TIP,
            uCallbackMessage: TRAY,
            hIcon: LoadIconW(
                Some(GetModuleHandleW(None).unwrap_or_default().into()),
                windows::core::PCWSTR(std::ptr::without_provenance(1)),
            )
            .or_else(|_| LoadIconW(None, IDI_APPLICATION))
            .unwrap_or_default(),
            ..Default::default()
        };
        for (target, value) in icon.szTip.iter_mut().zip("Carlitos".encode_utf16()) {
            *target = value;
        }
        Shell_NotifyIconW(action, &icon).as_bool()
    }
}
