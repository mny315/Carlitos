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

thread_local! {
    static HAPTIC_TICKS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

pub(super) fn record_haptic(kind: &str) {
    if kind == "scale-tick" {
        HAPTIC_TICKS.set(HAPTIC_TICKS.get() + 1);
    }
}

fn request(request: Value) -> Result<Value> {
    super::ui::WINDOW.with_borrow(|weak| {
        let w = weak.upgrade().context("No UI attached")?;
        let s = w.global::<State>();
        let t = w.global::<Theme>();
        match request["op"].as_str().unwrap_or("state") {
            "setup" => { s.set_language_index(2); s.set_theme_index(2); s.set_sort_index(0); s.set_text_size_percent(100); s.set_ui_scale_percent(100); t.set_reduced_motion(false); s.invoke_settings_changed(); }
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
                        "dismiss" => s.set_notice("".into()),
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
        snapshot["picker_owner"] = json!(s.get_picker_owner().as_str());
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
