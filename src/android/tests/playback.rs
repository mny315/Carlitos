//! Instrumentation-only entry point, enabled only in the isolated test package.
use crate::app::Command;
use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::jstring,
};
use serde_json::{Value, json};

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_mny315_carlitos_PlaybackSuite_nativeRequest(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    request: JString<'_>,
) -> jstring {
    let result = (|| -> anyhow::Result<Value> {
        anyhow::ensure!(
            super::context()
                .files
                .to_string_lossy()
                .contains("/io.github.mny315.carlitos.playbacktest/"),
            "Playback tests require the isolated playbacktest package"
        );
        let value = serde_json::from_str(&String::from(env.get_string(&request)?))?;
        let (tx, rx) = std::sync::mpsc::channel();
        super::runtime::send(Command::PlaybackTest(value, tx));
        rx.recv_timeout(std::time::Duration::from_secs(10))?
    })();
    let value = match result {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    };
    env.new_string(value.to_string())
        .map_or(std::ptr::null_mut(), |s| s.into_raw())
}
