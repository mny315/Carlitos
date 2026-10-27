use super::audio::{Audio, Playback};
use super::{Command, Controller, Event, Handle, Options, demo_library, text};
use crate::{
    import::ScanControl,
    library::{Library, Target},
    storage::Store,
};
use anyhow::{Context, Result};
use std::{sync::mpsc, time::Instant};

impl Handle {
    pub fn start(options: Options) -> Result<Self> {
        #[cfg(target_os = "android")]
        return crate::android::runtime::attach(options);
        #[cfg(not(target_os = "android"))]
        Self::start_worker(options)
    }
    #[cfg(target_os = "android")]
    pub(crate) fn client(
        tx: mpsc::Sender<Command>,
        events: async_channel::Receiver<Event>,
    ) -> Self {
        Self {
            tx,
            events,
            thread: None,
        }
    }
    pub(crate) fn start_worker(options: Options) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (events_tx, events) = async_channel::unbounded();
        let sender = tx.clone();
        let thread = std::thread::Builder::new()
            .name("carlitos-controller".into())
            .spawn(move || {
                let mut store = if options.demo {
                    None
                } else {
                    Some(Store::open(&options.database))
                };
                let mut load_error = None;
                let library = if options.demo {
                    demo_library()
                } else {
                    match store.as_ref().unwrap() {
                        Ok(s) => s.load(),
                        Err(e) => Err(anyhow::anyhow!("{e:#}")),
                    }
                    .unwrap_or_else(|e| {
                        let _ = events_tx.try_send(Event::Notice(format!("{e:#}")));
                        load_error = Some(e);
                        Library::default()
                    })
                };
                if let Some(error) = load_error {
                    store = Some(Err(error));
                }
                let audio = Audio::start(sender.clone(), options.fake_audio);
                let mut app = Controller {
                    store,
                    library,
                    audio,
                    playback: Playback::default(),
                    requested_rate: options.settings.playback_rate,
                    pending_rate: None,
                    requested_skip_silence: options.settings.skip_silence,
                    tx: sender,
                    events: events_tx,
                    token: 0,
                    scan_generation: 0,
                    scan_control: ScanControl::default(),
                    scan_source: None,
                    scans: vec![],
                    drafts: vec![],
                    import_folders: vec![],
                    issues: vec![],
                    checkpoint: Instant::now(),
                    options,
                    quitting: false,
                    pending_save: false,
                    maintenance: false,
                    maintenance_control: ScanControl::default(),
                    tag_control: ScanControl::default(),
                    sync: None,
                    sync_hashing: false,
                    sync_hash_control: ScanControl::default(),
                    sync_hash_attempted: Default::default(),
                    restore_progress: None,
                };
                if let Err(error) = app.configure_sync() {
                    app.notice(format!("{error:#}"));
                }
                app.emit_library();
                if !app.options.demo {
                    app.refresh_missing_tags();
                }
                if !app.options.demo {
                    // Distinct command tokens keep the first acknowledgement
                    // from publishing/saving half-applied startup preferences.
                    if let Err(error) = app
                        .apply_rate(app.options.settings.playback_rate)
                        .and_then(|()| app.set_skip_silence(app.options.settings.skip_silence))
                    {
                        app.notice(format!("{error:#}"));
                    }
                }
                if app.options.demo {
                    app.playback = Playback {
                        position: app.library.session.position,
                        duration: app.library.active_file().and_then(|f| f.duration),
                        phase: crate::player::Phase::Ready,
                        rate: app.options.settings.playback_rate,
                        skip_silence: app.options.settings.skip_silence,
                        ..Default::default()
                    };
                    let _ = app.events.try_send(Event::Playback(app.playback.clone()));
                }
                if !app.options.demo
                    && let Some(Target::Book(id)) = app.library.session.current.clone()
                {
                    let position = app.library.session.position;
                    if let Err(e) = app.load(id, position, false) {
                        app.notice(format!("{e:#}"));
                    }
                }
                let mut background_error = None;
                while let Ok(command) = rx.recv() {
                    let background = matches!(
                        command,
                        Command::Playback(_)
                            | Command::AudioFailed(..)
                            | Command::AudioAbsent(_)
                            | Command::AudioStopped
                    );
                    let checkpoint = app.checkpoint;
                    let result = app.handle(command);
                    if app.checkpoint != checkpoint {
                        background_error = None;
                    }
                    match result {
                        Ok(true) => break,
                        Ok(false) => {}
                        Err(e) => {
                            let message = format!("{e:#}");
                            // Repeated audio snapshots retry a failed write.
                            // Report once until a checkpoint succeeds, so a full
                            // disk cannot keep raising a window the user hid.
                            // Explicit actions, including Quit, still report.
                            let report = !background
                                || app.quitting
                                || background_error.as_ref() != Some(&message);
                            app.quitting = false;
                            if report {
                                let _ = app.events.try_send(Event::Show);
                                app.notice(message.clone());
                            }
                            if background {
                                background_error = Some(message);
                            }
                        }
                    }
                }
                app.scan_control.cancel();
                app.maintenance_control.cancel();
                app.tag_control.cancel();
                app.sync_hash_control.cancel();
                app.sync.take();
                for scan in app.scans {
                    let _ = scan.join();
                }
                app.audio.stop();
            })
            .context(text(
                "Не удалось запустить обработку библиотеки",
                "Could not start the library worker",
            ))?;
        Ok(Self {
            tx,
            events,
            thread: Some(thread),
        })
    }
    pub fn join(&mut self) {
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Controller {
    pub(super) fn reap_workers(&mut self) {
        // Completed cover/hash jobs must not retain OS thread handles until
        // the next import, especially across a long desktop session.
        self.scans.retain(|worker| !worker.is_finished());
    }
    pub(super) fn finish(&mut self) -> Result<()> {
        // An unreadable database is never replaced by the empty fallback view.
        if self.store.as_ref().is_some_and(|store| store.is_ok()) {
            self.save()?;
        }
        if !self.options.demo && self.options.settings_writable {
            self.options.settings.save(&self.options.settings_path)?;
        }
        self.audio.stop();
        self.sync_hash_control.cancel();
        if let Some(sync) = &self.sync {
            sync.flush();
        }
        self.sync.take();
        for scan in self.scans.drain(..) {
            let _ = scan.join();
        }
        let _ = self.events.try_send(Event::Quit);
        Ok(())
    }
}
