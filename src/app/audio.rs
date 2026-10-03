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
        #[cfg(target_os = "android")]
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
