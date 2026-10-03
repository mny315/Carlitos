use crate::app::{
    Command,
    audio::{AudioCommand, Playback},
};
use anyhow::{Context, Result};
use jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JClass, JObject, JString, JValue},
};
use serde_json::{Value, json};
use std::sync::{OnceLock, mpsc::Sender};

struct Bridge {
    vm: JavaVM,
    class: GlobalRef,
}
static BRIDGE: OnceLock<Bridge> = OnceLock::new();
static SENDER: OnceLock<Sender<Command>> = OnceLock::new();
pub fn set_sender(sender: Sender<Command>) {
    let _ = SENDER.set(sender);
}
fn send(command: Command) {
    if let Some(sender) = SENDER.get() {
        let _ = sender.send(command);
    } else {
        super::runtime::send(command);
    }
}
pub fn dispatch(value: Value) -> Result<()> {
    let bridge = BRIDGE.get().context("Android bridge not initialized")?;
    let mut env = bridge.vm.attach_current_thread()?;
    // UI threads remain attached across many slider/haptic events.
    env.with_local_frame(8, |env| -> Result<()> {
        let message = env.new_string(value.to_string())?;
        let class: &JClass<'_> = bridge.class.as_obj().into();
        let result = env.call_static_method(
            class,
            "dispatch",
            "(Ljava/lang/String;)V",
            &[JValue::Object(&message)],
        );
        if env.exception_check()? {
            env.exception_describe()?;
            env.exception_clear()?;
        }
        result?;
        Ok(())
    })
}
pub fn documents(value: Value) -> Result<Value> {
    let bridge = BRIDGE.get().context("Android bridge not initialized")?;
    let mut env = bridge.vm.attach_current_thread()?;
    // Attached Rust workers are long-lived: release local JNI refs per request.
    env.with_local_frame(16, |env| -> Result<Value> {
        let message = env.new_string(value.to_string())?;
        let class: &JClass<'_> = bridge.class.as_obj().into();
        let result = env.call_static_method(
            class,
            "documents",
            "(Ljava/lang/String;)Ljava/lang/String;",
            &[JValue::Object(&message)],
        );
        if env.exception_check()? {
            env.exception_describe()?;
            env.exception_clear()?;
        }
        let result = JString::from(result?.l()?);
        let value: Value = serde_json::from_str(&String::from(env.get_string(&result)?))?;
        if let Some(error) = value["error"].as_str() {
            anyhow::bail!("{error}");
        }
        Ok(value["value"].clone())
    })
}
pub fn audio(command: AudioCommand) -> Result<()> {
    let value = match command {
        AudioCommand::Load {
            token,
            uri,
            position,
            playing,
            title,
            rate,
            skip_silence,
        } => {
            json!({"op":"load","token":token,"uri":uri,"position":position,"playing":playing,"title":title,"rate":rate,"silence":skip_silence})
        }
        AudioCommand::Playing(token, playing) => {
            json!({"op":"playing","token":token,"playing":playing})
        }
        AudioCommand::Seek(token, position) => {
            json!({"op":"seek","token":token,"position":position})
        }
        AudioCommand::Volume(volume, muted) => json!({"op":"volume","volume":volume,"muted":muted}),
        AudioCommand::Rate(token, rate) => json!({"op":"rate","token":token,"rate":rate}),
        AudioCommand::RateStep(token, forward) => {
            json!({"op":"rate_step","token":token,"forward":forward})
        }
        AudioCommand::SkipSilence(token, enabled) => {
            json!({"op":"silence","token":token,"enabled":enabled})
        }
        AudioCommand::Snapshot => json!({"op":"snapshot"}),
        AudioCommand::Stop(token) => json!({"op":"stop","token":token}),
        AudioCommand::Quit => return Ok(()),
    };
    dispatch(value)
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_mny315_carlitos_Bridge_initialize(
    mut env: JNIEnv<'_>,
    class: JClass<'_>,
    context: JObject<'_>,
) {
    let result = (|| -> Result<()> {
        let _ = BRIDGE.set(Bridge {
            vm: env.get_java_vm()?,
            class: env.new_global_ref(class)?,
        });
        super::initialize_context(&mut env, &context)?;
        let (options, warning) = super::options();
        super::runtime::initialize(options)?;
        if let Some(warning) = warning {
            // The first Activity may not exist yet. The runtime retains the
            // recovery/read-only notice until its UI subscribes.
            super::runtime::send(Command::Error(warning));
        }
        Ok(())
    })();
    if let Err(error) = result {
        let _ = env.throw_new("java/lang/IllegalStateException", format!("{error:#}"));
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_mny315_carlitos_Bridge_event(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    message: JString<'_>,
) {
    let result = (|| -> Result<()> {
        let value: Value = serde_json::from_str(&String::from(env.get_string(&message)?))?;
        let number = |key: &str| value[key].as_u64().unwrap_or(0);
        let flag = |key: &str| value[key].as_bool().unwrap_or(false);
        match value["op"].as_str().context("Missing Android event type")? {
            "back" => super::ui::back(),
            "configuration" => super::ui::configuration(super::Configuration {
                language: value["language"].as_str().unwrap_or("en").into(),
                dark: flag("dark"),
                font_scale: value["font_scale"].as_f64().unwrap_or(1.0) as f32,
            }),
            "playback" => {
                use crate::player::Phase;
                send(Command::Playback(Playback {
                    token: number("token"),
                    position: number("position"),
                    duration: value["duration"].as_u64(),
                    playing: flag("playing"),
                    rate: value["rate"].as_f64().unwrap_or(1.0),
                    skip_silence: flag("silence"),
                    phase: match value["phase"].as_str() {
                        Some("ready") => Phase::Ready,
                        Some("loading") => Phase::Loading,
                        Some("seeking") => Phase::Seeking,
                        Some("error") => Phase::Error,
                        _ => Phase::Empty,
                    },
                    seekable: flag("seekable"),
                    seek_done: flag("seek_done"),
                    ended: flag("ended"),
                    barrier: flag("barrier"),
                    error: value["error"].as_str().map(str::to_owned),
                }));
            }
            "document" => {
                let doc: super::documents::Document =
                    serde_json::from_value(value["document"].clone())?;
                let name = doc.name.clone();
                send(Command::OpenDocument(doc.media(name)?));
            }
            "picked" => send(Command::Picked(
                value["kind"]
                    .as_str()
                    .context("Missing picker kind")?
                    .into(),
                value["uri"].as_str().context("Missing picker URI")?.into(),
                value["name"].as_str().unwrap_or_default().into(),
                value["owner"].as_str().unwrap_or_default().into(),
            )),
            "play" => send(Command::Playing(flag("value"))),
            "seek" => send(Command::SeekAbsolute(number("position"))),
            "delta" => send(Command::SeekDelta(value["value"].as_i64().unwrap_or(0))),
            "next" => send(Command::Next(flag("value"))),
            "stop" => send(Command::Stop),
            "rate" => send(Command::Rate(value["value"].as_f64().unwrap_or(1.0))),
            "volume" => send(Command::Volume(value["value"].as_f64().unwrap_or(0.7))),
            "hidden" => send(Command::Hidden(flag("value"))),
            "error" => send(Command::Error(
                value["message"]
                    .as_str()
                    .unwrap_or("Android error")
                    .to_owned(),
            )),
            "service_stopped" => send(Command::AudioStopped),
            "audio_failed" => send(Command::AudioFailed(
                number("token"),
                value["message"]
                    .as_str()
                    .unwrap_or("Android audio unavailable")
                    .into(),
            )),
            _ => anyhow::bail!("Unknown Android event"),
        }
        Ok(())
    })();
    if let Err(error) = result {
        send(Command::Error(format!("Android callback: {error:#}")));
    }
}
