use super::{check, key};
use crate::{
    AppWindow, State,
    app::{Command, view::View},
};
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};

pub(super) fn platform_checks() {
    let window = AppWindow::new().expect("platform shortcut window");
    let (tx, received) = std::sync::mpsc::channel();
    let (_view, _covers) = View::new(&window, tx, Default::default());
    let state = window.global::<State>();
    window.invoke_focus_shell();
    let quit_key = || {
        window.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        key(&window, "q");
        window.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
    };
    quit_key();
    check(
        received.try_iter().any(|c| matches!(c, Command::Quit)),
        "desktop Ctrl+Q reaches the controller",
    );
    state.set_android(true);
    window.invoke_focus_shell();
    quit_key();
    check(
        !received.try_iter().any(|c| matches!(c, Command::Quit)),
        "Android hardware Ctrl+Q cannot shut down its persistent controller",
    );
}
