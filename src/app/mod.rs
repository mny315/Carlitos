pub mod audio;
mod catalog;
mod commands;
mod demo;
mod messages;
// Exercise the platform-independent Android event buffer on the host too.
#[cfg(all(test, not(target_os = "android")))]
#[path = "../android/runtime/pending.rs"]
mod android_events;
mod playback;
#[cfg(all(target_os = "android", feature = "android-playback-tests"))]
#[path = "tests/device.rs"]
mod playback_device_tests;
mod scanning;
pub use demo::demo_library;
#[cfg(test)]
mod tests;
mod theme;
mod ui;
pub mod view;
mod worker;
use crate::{
    import::ScanControl,
    library::*,
    settings::Settings,
    storage::{Draft, Store},
};
use anyhow::{Result, bail};
use audio::{Audio, Playback};
use std::{path::PathBuf, sync::mpsc::Sender, time::Instant};
pub use ui::Ui;

pub use messages::{Command, Event};

pub struct Options {
    pub database: PathBuf,
    pub settings_path: PathBuf,
    pub settings: Settings,
    pub settings_writable: bool,
    pub demo: bool,
    pub fake_audio: bool,
}
pub struct Handle {
    pub tx: Sender<Command>,
    pub events: async_channel::Receiver<Event>,
    thread: Option<std::thread::JoinHandle<()>>,
}
struct Controller {
    store: Option<Result<Store>>,
    library: Library,
    audio: Audio,
    playback: Playback,
    // A load can overtake the snapshot that acknowledges a preference change.
    // Keep requested audio parameters separate from confirmed, saved settings.
    requested_rate: f64,
    requested_skip_silence: bool,
    tx: Sender<Command>,
    events: async_channel::Sender<Event>,
    token: u64,
    scan_generation: u64,
    scan_control: ScanControl,
    scan_source: Option<Id>,
    scans: Vec<std::thread::JoinHandle<()>>,
    drafts: Vec<Draft>,
    issues: Vec<String>,
    checkpoint: Instant,
    options: Options,
    quitting: bool,
    pending_save: bool,
    maintenance: bool,
    maintenance_control: ScanControl,
    tag_control: ScanControl,
}
impl Controller {
    fn notice(&self, text: String) {
        let _ = self.events.try_send(Event::Notice(text));
    }
    fn emit_library(&self) {
        let _ = self
            .events
            .try_send(Event::Library(Box::new(self.library.clone())));
    }
    fn emit_drafts(&self) {
        let _ = self
            .events
            .try_send(Event::Drafts(self.drafts.clone(), self.issues.clone()));
    }
    fn store(&mut self) -> Result<&mut Store> {
        match self.store.as_mut() {
            Some(Ok(s)) => Ok(s),
            Some(Err(e)) => bail!("{e:#}"),
            None => bail!(
                "{}",
                text("Это демонстрационная библиотека", "This is a demo library")
            ),
        }
    }
    fn save(&mut self) -> Result<()> {
        if self.options.demo {
            return Ok(());
        }
        let session = self.library.session.clone();
        let progress = self.library.progress.clone();
        self.store()?.save(&session, &progress)?;
        self.checkpoint = Instant::now();
        Ok(())
    }
    fn reconcile(&mut self, mut library: Library) {
        if self
            .library
            .session
            .current
            .as_ref()
            .is_some_and(|t| library.file_for(t).is_some())
        {
            library.session = self.library.session.clone();
        } else {
            self.stop();
            library.session.current = None;
            library.session.position = 0;
        }
        self.library = library;
        self.emit_library();
    }
}
pub fn text<'a>(ru: &'a str, en: &'a str) -> &'a str {
    if crate::i18n::english() { en } else { ru }
}
