//! Test-only inspection of real Slint geometry. Touches are injected by Android.
use crate::{State, Theme};
use anyhow::{Context, Result};
use i_slint_backend_testing::{ElementHandle, ElementRoot};
use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::jstring,
};
use serde_json::{Value, json};
use slint::{ComponentHandle, Model};
use std::sync::atomic::{AtomicU32, Ordering};

static BACKDROP_SAMPLES: AtomicU32 = AtomicU32::new(0);
pub(crate) fn record_backdrop_sample() {
    BACKDROP_SAMPLES.fetch_add(1, Ordering::Relaxed);
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_mny315_carlitos_UiSuite_nativeBackdropFrames(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jni::sys::jlong {
    i64::from(BACKDROP_SAMPLES.load(Ordering::Relaxed))
}

#[path = "../../../tests/support/animation.rs"]
mod animation;

thread_local! {
    static HAPTIC_TICKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static HAPTIC_BUTTONS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static ANIMATION_REPORT: std::cell::RefCell<Value> = const { std::cell::RefCell::new(Value::Null) };
}

pub(super) fn record_haptic(kind: &str) {
    if kind == "scale-tick" {
        HAPTIC_TICKS.set(HAPTIC_TICKS.get() + 1);
    } else if kind == "button" || kind == "playback" {
        HAPTIC_BUTTONS.set(HAPTIC_BUTTONS.get() + 1);
    }
}

fn request(request: Value) -> Result<Value> {
    super::ui::WINDOW.with_borrow(|weak| {
        let w = weak.upgrade().context("No UI attached")?;
        let s = w.global::<State>();
        let t = w.global::<Theme>();
        match request["op"].as_str().unwrap_or("state") {
            "animation-start" => {
                ANIMATION_REPORT.with_borrow_mut(|r| *r = Value::Null);
                animation::start(&w, |report| ANIMATION_REPORT.with_borrow_mut(|r| *r = report));
            }
            "animation-report" => return Ok(json!({"report":ANIMATION_REPORT.with_borrow(Clone::clone)})),
            "setup" => { s.set_language_index(2); s.set_theme_index(2); s.set_sort_index(0); s.set_text_size_percent(100); s.set_ui_scale_percent(100); t.set_reduced_motion(false); s.invoke_settings_changed(); }
            "appearance" => {
                s.set_theme_index(request["theme"].as_i64().unwrap_or(0) as i32);
                s.set_language_index(request["language"].as_i64().unwrap_or(2) as i32);
                s.set_text_size_percent(request["text_percent"].as_i64().unwrap_or(100) as i32);
                s.invoke_settings_changed();
            }
            "action" => s.invoke_action(request["name"].as_str().unwrap_or("").into(), request["arg"].as_str().unwrap_or("").into()),
            "page" => s.invoke_navigate(request["value"].as_i64().unwrap_or(0) as i32),
            "feedback" => {
                use crate::app::Event;
                super::ui::VIEW.with_borrow(|weak| -> Result<()> {
                    let view = weak.upgrade().context("No view attached")?;
                    let mut view = view.borrow_mut();
                    match request["kind"].as_str().unwrap_or("") {
                        "scan" => view.event(Event::Scanning(true, "Scanning…".into())),
                        "scan-done" => view.event(Event::Scanning(false, String::new())),
                        "imported" => {
                            view.event(Event::Notice("Books added to your library".into()));
                            view.event(Event::Imported);
                        }
                        "updated" => view.event(Event::SourceUpdated(0, "Source updated".into())),
                        "error" => view.event(Event::Notice("Test error".into())),
                        "dismiss" => s.invoke_dismiss_notice(),
                        kind => anyhow::bail!("Unknown feedback: {kind}"),
                    }
                    Ok(())
                })?;
            }
            "elements" => {
                let mut elements = vec![];
                w.root_element().visit_descendants(|e: ElementHandle| {
                    if let Some(label) = e.accessible_label().filter(|s| !s.is_empty()) {
                        let pos = e.absolute_position(); let size = e.size();
                        elements.push(json!({"label":label.as_str(), "type":e.type_name().map(|s|s.to_string()),
                            "x":pos.x,"y":pos.y,"w":size.width,"h":size.height,
                            "role":format!("{:?}", e.accessible_role()),
                            "value":e.accessible_value().map(|value|value.to_string())}));
                    }
                    std::ops::ControlFlow::<()>::Continue(())
                });
                return Ok(json!(elements));
            }
            "state" => {},
            "background" => {
                if let Some(enabled) = request["enabled"].as_bool() { s.set_animated_background(enabled); }
                if let Some(reduced) = request["reduced"].as_bool() { t.set_reduced_motion(reduced); }
                s.invoke_settings_changed();
            }
            op => anyhow::bail!("Unknown UI test operation {op}"),
        }
        let size = w.window().size().to_logical(w.window().scale_factor());
        let mut snapshot = json!({"page":s.get_page(),"filter":s.get_filter(),"overlay":s.get_overlay(),"menu":s.get_book_menu(),
            "query":s.get_query().as_str(),"title":s.get_edit_title().as_str(),"books_scroll_y":s.get_books_scroll_y(),
            "edit_key":s.get_edit_key().as_str(),"cover_path":s.get_edit_cover_path().as_str(),"cover_loading":s.get_cover_loading(),
            "single_book":s.get_single_book(),
            "drafts":s.get_drafts().row_count(),"selected_draft":s.get_selected_draft(),
            "draft_title":s.get_draft_title().as_str(),"draft_files":s.get_draft_files().row_count(),
            "source_key":s.get_relocate_key().as_str(),"source_path":s.get_relocate_path().as_str(),
            "source_busy":s.get_source_busy(),"source_error":s.get_source_error().as_str(),
            "keyboard":s.get_keyboard_visible(),"short":s.get_short_screen(),
            "selected":s.get_selected().key.as_str(),"parts":s.get_parts().row_count(),
            "part_rows":s.get_parts().iter().map(|p|json!({"key":p.key.as_str(),"title":p.title.as_str(),"active":p.active,"chapter":p.chapter})).collect::<Vec<_>>(),
            "books":s.get_books().iter().map(|b|json!({"key":b.key.as_str(),"title":b.title.as_str()})).collect::<Vec<_>>(),
            "playing":s.get_playing(),"position":s.get_position(),"volume":s.get_volume(),"muted":s.get_muted(),"rate":s.get_rate(),
            "dark":t.get_dark(),"english":t.get_english(),"font_scale":t.get_font_scale(),
            "text_size_percent":s.get_text_size_percent(),"ui_scale_percent":s.get_ui_scale_percent(),
            "haptic_ticks":HAPTIC_TICKS.get(),"notice":s.get_notice().as_str(),"scanning":s.get_scanning(),
            "scale":w.window().scale_factor(),"width":size.width,"height":size.height});
        snapshot["notice_timed"] = json!(s.get_notice_timed());
        snapshot["haptic_buttons"] = json!(HAPTIC_BUTTONS.get());
        snapshot["notice_remaining"] = json!(s.get_notice_remaining());
        snapshot["picker_owner"] = json!(s.get_picker_owner().as_str());
        snapshot["loading"] = json!(s.get_loading());
        snapshot["background_enabled"] = json!(s.get_animated_background());
        snapshot["background_running"] = json!(s.get_backdrop_running());
        snapshot["ui_visible"] = json!(s.get_ui_visible());
        snapshot["sync_enabled"] = json!(s.get_local_sync());
        snapshot["sync_manual"] = json!(s.get_sync_manual());
        snapshot["sync_invitation"] = json!(!s.get_sync_invitation().is_empty());
        snapshot["sync_peers"] = json!(s.get_sync_peers().row_count());
        snapshot["sync_message"] = json!(s.get_sync_message().as_str());
        snapshot["sync_busy"] = json!(s.get_sync_busy());
        snapshot["sync_indexing"] = json!(s.get_sync_indexing());
        snapshot["source_selection"] = json!(s.get_source_selection());
        snapshot["selected_source_count"] = json!(s.get_selected_source_count());
        snapshot["remove_source_count"] = json!(s.get_remove_source_count());
        snapshot["sources"] = json!(s.get_sources().iter().map(|row| json!({
            "key":row.key.as_str(),"path":row.path.as_str(),"selected":row.selected
        })).collect::<Vec<_>>());
        Ok(snapshot)
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_mny315_carlitos_UiSuite_nativeRequest(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    input: JString<'_>,
) -> jstring {
    let result = (|| -> Result<Value> {
        anyhow::ensure!(
            super::context()
                .files
                .to_string_lossy()
                .contains("/io.github.mny315.carlitos.playbacktest/"),
            "UI tests require isolated package"
        );
        let input = serde_json::from_str(&String::from(env.get_string(&input)?))?;
        let (tx, rx) = std::sync::mpsc::channel();
        slint::invoke_from_event_loop(move || {
            let _ = tx.send(request(input));
        })?;
        rx.recv_timeout(std::time::Duration::from_secs(10))?
    })();
    let value = result.unwrap_or_else(|e| json!({"error":format!("{e:#}")}));
    env.new_string(value.to_string())
        .map_or(std::ptr::null_mut(), |s| s.into_raw())
}
