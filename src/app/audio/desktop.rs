use super::{Audio, AudioCommand, Playback};
use crate::{
    app::{Command, text},
    player::{EventKind, Phase, Player},
};
use std::{sync::mpsc::Sender, time::Duration};

impl Audio {
    pub fn start(app: Sender<Command>, fake: bool) -> Self {
        let (tx, rx) = async_channel::unbounded();
        let startup = app.clone();
        let thread = std::thread::Builder::new()
            .name("carlitos-audio".into())
            .spawn(move || {
                let finished = app.clone();
                #[cfg(target_os = "linux")]
                let context = gst::glib::MainContext::new();
                let run = async move {
                    let (events_tx, events_rx) = async_channel::unbounded();
                    let mut player = match Player::new(events_tx, fake) {
                        Ok(player) => player,
                        Err(e) => {
                            let _ = app.send(Command::Error(format!("{e:#}")));
                            return;
                        }
                    };
                    let mut token = 0;
                    let mut failure = None;
                    let mut pending_barrier: Option<std::time::Instant> = None;
                    #[cfg(windows)]
                    let mut last_snapshot = std::time::Instant::now();
                    enum Input {
                        Command(Result<AudioCommand, async_channel::RecvError>),
                        Event(Result<crate::player::Event, async_channel::RecvError>),
                        Tick,
                    }
                    loop {
                        let commands = async { Input::Command(rx.recv().await) };
                        let events = async { Input::Event(events_rx.recv().await) };
                        let tick = async {
                            if pending_barrier.is_some() {
                                delay(Duration::from_millis(50)).await;
                            } else if player.playing && player.phase == Phase::Ready {
                                delay(Duration::from_millis(if cfg!(windows) { 20 } else { 250 }))
                                    .await;
                            } else {
                                std::future::pending::<()>().await;
                            }
                            Input::Tick
                        };
                        let input = futures_lite::future::race(
                            commands,
                            futures_lite::future::race(events, tick),
                        )
                        .await;
                        #[cfg(windows)]
                        let is_tick = matches!(input, Input::Tick);
                        let mut seek_done = false;
                        let mut error = None;
                        let mut barrier = false;
                        let outcome = match input {
                            Input::Command(Ok(AudioCommand::Load {
                                token: next,
                                uri,
                                position,
                                playing,
                            })) => {
                                token = next;
                                player.load(&uri, position, playing)
                            }
                            Input::Command(Ok(AudioCommand::Playing(next, playing))) => {
                                token = next;
                                player.poll();
                                player.set_playing(playing)
                            }
                            Input::Command(Ok(AudioCommand::Seek(next, position))) => {
                                token = next;
                                player.seek(position)
                            }
                            Input::Command(Ok(AudioCommand::Volume(volume, muted))) => {
                                player.volume(volume, muted);
                                Ok(())
                            }
                            Input::Command(Ok(AudioCommand::Rate(next, rate))) => {
                                token = next;
                                player.set_rate(rate)
                            }
                            Input::Command(Ok(AudioCommand::RateStep(next, forward))) => {
                                token = next;
                                player.set_rate(crate::player::stepped_rate(player.rate(), forward))
                            }
                            Input::Command(Ok(AudioCommand::SkipSilence(next, enabled))) => {
                                token = next;
                                player.set_skip_silence(enabled)
                            }
                            Input::Command(Ok(AudioCommand::Stop(next))) => {
                                token = next;
                                player.stop();
                                Ok(())
                            }
                            Input::Command(Ok(AudioCommand::Snapshot)) => {
                                pending_barrier = Some(std::time::Instant::now());
                                player.poll();
                                Ok(())
                            }
                            Input::Command(Ok(AudioCommand::Quit) | Err(_)) => {
                                player.poll();
                                player.stop();
                                break;
                            }
                            Input::Event(Ok(event)) => {
                                if event.generation != player.generation {
                                    continue;
                                }
                                if let EventKind::Error(ref message) = event.kind {
                                    error = Some(message.clone());
                                }
                                match player.handle(&event) {
                                    Ok(done) => {
                                        seek_done = done;
                                        Ok(())
                                    }
                                    Err(e) => Err(e),
                                }
                            }
                            Input::Event(Err(_)) => break,
                            Input::Tick => {
                                player.poll();
                                Ok(())
                            }
                        };
                        if let Err(e) = outcome {
                            error = Some(format!("{e:#}"));
                        }
                        // A newer command can make the controller discard
                        // the first error snapshot. Retain the failure until
                        // loading another file or stopping clears the state.
                        if player.phase == Phase::Error {
                            if error.is_some() {
                                failure = error;
                            }
                            error = failure.clone();
                        } else {
                            failure = None;
                        }
                        if pending_barrier.is_some_and(|started| {
                            !matches!(player.phase, Phase::Loading | Phase::Seeking)
                                || started.elapsed() >= Duration::from_secs(2)
                        }) {
                            barrier = true;
                            pending_barrier = None;
                        }
                        seek_done |= player.take_silence_skip();
                        #[cfg(windows)]
                        {
                            if is_tick
                                && !barrier
                                && !seek_done
                                && error.is_none()
                                && !player.ended
                                && last_snapshot.elapsed() < Duration::from_millis(250)
                            {
                                continue;
                            }
                            last_snapshot = std::time::Instant::now();
                        }
                        let snapshot = Playback {
                            token,
                            position: player.seek_base(),
                            duration: player.duration,
                            playing: player.playing,
                            rate: player.rate(),
                            skip_silence: player.skip_silence(),
                            phase: player.phase,
                            seekable: player.seekable,
                            seek_done,
                            // The controller may reject a snapshot after a
                            // newer command. Keep EOS until a load or seek.
                            ended: player.ended,
                            barrier,
                            error,
                        };
                        if app.send(Command::Playback(snapshot)).is_err() {
                            break;
                        }
                    }
                };
                #[cfg(target_os = "linux")]
                if let Err(e) = context.with_thread_default(|| context.block_on(run)) {
                    eprintln!("Audio context: {e}");
                }
                #[cfg(windows)]
                futures_lite::future::block_on(run);
                let _ = finished.send(Command::AudioStopped);
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                // Keep the controller alive so the library and Quit remain
                // usable even when the OS cannot start the audio worker.
                let _ = startup.send(Command::Error(format!(
                    "{}: {error}",
                    text("Не удалось запустить аудиодвижок", "Could not start audio")
                )));
                let _ = startup.send(Command::AudioStopped);
                None
            }
        };
        Self { tx, thread }
    }
}

async fn delay(duration: Duration) {
    #[cfg(target_os = "linux")]
    gst::glib::timeout_future(duration).await;
    #[cfg(windows)]
    async_io::Timer::after(duration).await;
}
