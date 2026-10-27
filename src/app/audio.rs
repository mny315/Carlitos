#[cfg(target_os = "android")]
mod android;
#[cfg(any(target_os = "linux", windows))]
mod desktop;

use crate::player::Phase;

pub enum AudioCommand {
    Load {
        token: u64,
        uri: String,
        position: u64,
        playing: bool,
        #[cfg(target_os = "android")]
        title: String,
        rate: f64,
        #[cfg(target_os = "android")]
        skip_silence: bool,
    },
    Playing(u64, bool),
    Seek(u64, u64),
    Volume(f64, bool),
    Rate(u64, f64),
    RateStep(u64, bool),
    SkipSilence(u64, bool),
    Snapshot,
    Stop(u64),
    #[cfg(any(target_os = "linux", windows))]
    Barrier(std::sync::mpsc::Sender<()>),
    Quit,
}
#[derive(Clone, Debug)]
pub struct Playback {
    pub token: u64,
    pub position: u64,
    pub duration: Option<u64>,
    pub playing: bool,
    pub rate: f64,
    pub skip_silence: bool,
    pub phase: Phase,
    pub seekable: bool,
    pub seek_done: bool,
    pub ended: bool,
    pub barrier: bool,
    pub error: Option<String>,
    /// Set by the controller for a paused restore whose rounded decoder clock
    /// must not replace the confirmed book checkpoint in the view.
    pub preserve_progress: bool,
}
impl Default for Playback {
    fn default() -> Self {
        Self {
            token: 0,
            position: 0,
            duration: None,
            playing: false,
            rate: 1.0,
            skip_silence: false,
            phase: Phase::Empty,
            seekable: false,
            seek_done: false,
            ended: false,
            barrier: false,
            error: None,
            preserve_progress: false,
        }
    }
}
pub struct Audio {
    tx: async_channel::Sender<AudioCommand>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Audio {
    pub fn send(&self, command: AudioCommand) {
        let _ = self.tx.try_send(command);
    }
    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
    #[cfg(any(target_os = "linux", windows))]
    pub(super) fn wait_for_commands(&mut self) -> anyhow::Result<()> {
        use anyhow::Context;
        let (reply, received) = std::sync::mpsc::channel();
        if self.tx.try_send(AudioCommand::Barrier(reply)).is_err() {
            // A failed worker may already be tearing down its decoder.
            self.stop();
            return Ok(());
        }
        match received.recv_timeout(std::time::Duration::from_secs(2)) {
            Ok(()) => Ok(()),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                self.stop();
                Ok(())
            }
            Err(error) => Err(error).context(super::text(
                "Не удалось дождаться остановки аудиодвижка. Файлы не удалены.",
                "Could not wait for audio to stop. No files were deleted.",
            )),
        }
    }
    pub fn stop(&mut self) {
        self.send(AudioCommand::Quit);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Audio {
    fn drop(&mut self) {
        self.stop();
    }
}
