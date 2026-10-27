//! Activity callbacks enter Slint's thread; no Activity or UI handle is kept in JNI.
mod back;
mod saved;
pub use saved::restore;

use crate::{AppWindow, State, Theme, app};
use slint::ComponentHandle;
use std::{
    cell::{Cell, RefCell},
    rc::Weak,
    time::{Duration, Instant},
};

thread_local! {
    pub(super) static VIEW: RefCell<Weak<RefCell<app::view::View>>> = RefCell::default();
    pub(super) static WINDOW: RefCell<slint::Weak<AppWindow>> = RefCell::default();
    static IME_HIDDEN: Cell<Option<Instant>> = const { Cell::new(None) };
}

pub fn attach(ui: &app::Ui) {
    BACK_REQUESTS.invalidate();
    VIEW.set(std::rc::Rc::downgrade(&ui.view));
    WINDOW.set(ui.window.as_weak());
    let window = ui.window.as_weak();
    i_slint_backend_android_activity::set_touch_filter(Some(Box::new(
        move |phase, id, position| {
            use i_slint_backend_android_activity::TouchPhase;
            let phase = match phase {
                TouchPhase::Started => 0,
                TouchPhase::Moved => 1,
                TouchPhase::Ended => 2,
                TouchPhase::Cancelled => 3,
            };
            window.upgrade().is_some_and(|window| {
                window.invoke_library_touch(phase, id, position.x, position.y)
            })
        },
    )));
    let theme = ui.window.global::<Theme>();
    theme.set_touch(true);
    theme.on_haptic(|kind| {
        #[cfg(feature = "android-ui-tests")]
        super::ui_tests::record_haptic(kind.as_str());
        let _ = super::bridge::dispatch(serde_json::json!({"op":"haptic", "kind":kind.as_str()}));
    });
}

pub fn detach(ui: &app::Ui) {
    BACK_REQUESTS.invalidate();
    ui.window.global::<State>().set_ui_visible(false);
    crate::visualizer::set_enabled(false);
    i_slint_backend_android_activity::set_touch_filter(None);
    // TextInput keeps an IME composition outside its bound text property.
    // Commit it on focus loss before saving the draft for the next Activity.
    ui.window.invoke_dismiss_keyboard();
    saved::save(ui);
    WINDOW.set(slint::Weak::default());
    VIEW.set(Weak::default());
}

static BACK_REQUESTS: back::BackRequests = back::BackRequests::new();
pub fn keyboard_hidden() {
    IME_HIDDEN.set(Some(Instant::now()));
}
pub fn back() {
    // NativeActivity can deliver the same Back through more than one input
    // route within a frame. Coalesce before changing focus or dismissing a modal.
    let Some(request) = BACK_REQUESTS.claim() else {
        return;
    };
    if slint::invoke_from_event_loop(move || {
        if !BACK_REQUESTS.is_current(request) {
            return;
        }
        slint::Timer::single_shot(Duration::from_millis(32), move || {
            // A delayed Back belongs to the Activity which received it. An
            // older timer must neither navigate a recreated UI nor release
            // the coalescing flag of that UI's own newer Back request.
            if !BACK_REQUESTS.is_current(request) {
                return;
            }
            // An IME-consumed key can also arrive in NativeActivity's queue
            // after the inset update. It must not navigate the page as well.
            let ime_consumed = IME_HIDDEN
                .get()
                .is_some_and(|t| t.elapsed() < Duration::from_millis(150));
            WINDOW.with_borrow(|weak| {
                if let Some(w) = weak.upgrade() {
                    if w.global::<State>().get_keyboard_visible() {
                        w.invoke_dismiss_keyboard();
                    } else if !ime_consumed && !w.invoke_mobile_back() {
                        let _ = super::bridge::dispatch(serde_json::json!({"op":"background"}));
                    }
                }
            });
            BACK_REQUESTS.release(request);
        })
    })
    .is_err()
    {
        BACK_REQUESTS.release(request);
    }
}

pub fn configuration(mut configuration: super::Configuration) {
    if !configuration.font_scale.is_finite() || configuration.font_scale <= 0.0 {
        configuration.font_scale = 1.0;
    }
    *super::context().configuration.lock().unwrap() = configuration;
    let _ = slint::invoke_from_event_loop(move || {
        VIEW.with_borrow(|weak| {
            if let Some(view) = weak.upgrade() {
                view.borrow_mut().android_configuration_changed();
            }
        });
    });
}
