pub(crate) mod bridge;
pub(crate) mod documents;
#[cfg(feature = "android-import-tests")]
#[path = "tests/import.rs"]
mod import_tests;
#[cfg(feature = "android-playback-tests")]
#[path = "tests/playback.rs"]
mod playback_tests;
pub(crate) mod runtime;
pub(crate) mod ui;
#[cfg(feature = "android-ui-tests")]
#[path = "tests/ui.rs"]
mod ui_tests;

use crate::{State, app, settings::Settings};
use anyhow::Result;
use jni::{
    JNIEnv,
    objects::{JObject, JString},
};
use slint::{ComponentHandle, android::AndroidApp};
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

pub(crate) struct AppContext {
    pub files: PathBuf,
    pub cache: PathBuf,
    pub configuration: Mutex<Configuration>,
}
#[derive(Clone)]
pub(crate) struct Configuration {
    pub language: String,
    pub dark: bool,
    pub font_scale: f32,
}
static CONTEXT: OnceLock<AppContext> = OnceLock::new();

pub(crate) fn context() -> &'static AppContext {
    CONTEXT
        .get()
        .expect("Android context initialized before shared code")
}

fn directory(env: &mut JNIEnv<'_>, context: &JObject<'_>, method: &str) -> Result<PathBuf> {
    let file = env
        .call_method(context, method, "()Ljava/io/File;", &[])?
        .l()?;
    let path = env
        .call_method(file, "getAbsolutePath", "()Ljava/lang/String;", &[])?
        .l()?;
    let path = PathBuf::from(String::from(env.get_string(&JString::from(path))?));
    anyhow::ensure!(
        path.is_absolute(),
        "Android context returned a relative path"
    );
    Ok(path)
}

fn initialize_context(env: &mut JNIEnv<'_>, application: &JObject<'_>) -> Result<()> {
    let files = directory(env, application, "getFilesDir")?;
    let cache = directory(env, application, "getCacheDir")?;
    let locale = env
        .call_static_method(
            "java/util/Locale",
            "getDefault",
            "()Ljava/util/Locale;",
            &[],
        )?
        .l()?;
    let language = env
        .call_method(locale, "getLanguage", "()Ljava/lang/String;", &[])?
        .l()?;
    let language = String::from(env.get_string(&JString::from(language))?);
    let resources = env
        .call_method(
            application,
            "getResources",
            "()Landroid/content/res/Resources;",
            &[],
        )?
        .l()?;
    let configuration = env
        .call_method(
            resources,
            "getConfiguration",
            "()Landroid/content/res/Configuration;",
            &[],
        )?
        .l()?;
    let dark = env.get_field(&configuration, "uiMode", "I")?.i()? & 0x30 == 0x20;
    let font_scale = env.get_field(&configuration, "fontScale", "F")?.f()?;
    let _ = CONTEXT.set(AppContext {
        files,
        cache,
        configuration: Mutex::new(Configuration {
            language,
            dark,
            font_scale,
        }),
    });
    Ok(())
}

#[unsafe(no_mangle)]
fn android_main(android: AndroidApp) {
    if let Err(error) = run(android) {
        eprintln!("Carlitos Android startup: {error:#}");
    }
}

fn options() -> (app::Options, Option<String>) {
    let (mut settings, warning, settings_writable) = Settings::load(&Settings::path());
    settings.close_to_tray = false;
    (
        app::Options {
            database: crate::library::data_dir().join("library.sqlite3"),
            settings_path: Settings::path(),
            settings,
            settings_writable,
            demo: false,
            fake_audio: false,
        },
        warning,
    )
}
fn run(android: AndroidApp) -> Result<()> {
    slint::android::init(android)?;
    let ui = app::Ui::new(options().0)?;
    ui::attach(&ui);
    ui.connect(|_| {})?;
    for (kind, callback) in [("folder", 0), ("cover", 1), ("source", 2)] {
        let tx = ui.app.tx.clone();
        let window = ui.window.as_weak();
        let picker = move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let owner = match kind {
                "cover" | "source" => app::view::renew_picker_owner(&window),
                _ => Default::default(),
            };
            if let Err(error) = bridge::dispatch(
                serde_json::json!({"op":"pick", "kind":kind, "owner":owner.as_str()}),
            ) {
                let _ = tx.send(app::Command::Error(format!("{error:#}")));
            }
        };
        match callback {
            0 => ui.window.global::<State>().on_choose_folder(picker),
            1 => ui.window.global::<State>().on_choose_cover(picker),
            _ => ui.window.global::<State>().on_choose_source_folder(picker),
        }
    }
    let result = ui.window.run();
    ui::detach(&ui);
    runtime::detach();
    result?;
    Ok(())
}
