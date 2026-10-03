use super::{Audio, AudioCommand};
use crate::app::Command;
use std::sync::mpsc::Sender;

impl Audio {
    pub fn start(app: Sender<Command>, _fake: bool) -> Self {
        let (tx, rx) = async_channel::unbounded();
        let failure = app.clone();
        let thread = std::thread::Builder::new()
            .name("carlitos-audio".into())
            .spawn(move || {
                crate::android::bridge::set_sender(app.clone());
                let mut token = 0;
                while let Ok(command) = rx.recv_blocking() {
                    if matches!(command, AudioCommand::Quit) {
                        break;
                    }
                    match &command {
                        AudioCommand::Load { token: next, .. }
                        | AudioCommand::Playing(next, _)
                        | AudioCommand::Seek(next, _)
                        | AudioCommand::Rate(next, _)
                        | AudioCommand::RateStep(next, _)
                        | AudioCommand::SkipSilence(next, _)
                        | AudioCommand::Stop(next) => token = *next,
                        _ => {}
                    }
                    if let Err(error) = crate::android::bridge::audio(command) {
                        let _ = app.send(Command::AudioFailed(
                            token,
                            format!("Android audio: {error:#}"),
                        ));
                    }
                }
                let _ = app.send(Command::AudioStopped);
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                let _ = failure.send(Command::Error(format!("Android audio worker: {error}")));
                let _ = failure.send(Command::AudioStopped);
                None
            }
        };
        Self { tx, thread }
    }
}
