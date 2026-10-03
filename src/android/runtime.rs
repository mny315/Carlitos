//! One controller and Store per process. UI subscriptions contain one wakeup;
//! snapshots are coalesced here even while NativeActivity is stopped/destroyed.
mod pending;
use crate::app::{Command, Event, Handle, Options};
use crate::{library::Library, settings::Settings};
use anyhow::Result;
use pending::PendingEvents;
use std::sync::{Arc, Mutex, OnceLock, mpsc::Sender};

#[derive(Default)]
struct Mailbox {
    library: Option<Box<Library>>,
    playback: Option<crate::app::audio::Playback>,
    settings: Option<Settings>,
    drafts: Option<(Vec<crate::storage::Draft>, Vec<String>)>,
    drafts_dirty: bool,
    library_dirty: bool,
    playback_dirty: bool,
    volume_dirty: bool,
    settings_dirty: bool,
    scanning: (bool, String),
    source_updating: bool,
    hidden: bool,
    status_dirty: bool,
    pending: PendingEvents,
    subscriber: Option<async_channel::Sender<Event>>,
}
struct Runtime {
    tx: Sender<Command>,
    mailbox: Arc<Mutex<Mailbox>>,
    _owner: Mutex<Handle>,
}
static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub fn initialize(options: Options) -> Result<()> {
    if RUNTIME.get().is_some() {
        return Ok(());
    }
    let settings = options.settings.clone();
    let handle = Handle::start_worker(options)?;
    let mailbox = Arc::new(Mutex::new(Mailbox {
        settings: Some(settings),
        ..Default::default()
    }));
    let events = handle.events.clone();
    let cache = mailbox.clone();
    std::thread::Builder::new()
        .name("carlitos-events".into())
        .spawn(move || {
            while let Ok(event) = events.recv_blocking() {
                let mut cache = cache.lock().unwrap();
                match event {
                    Event::Library(library) => {
                        if cache
                            .library
                            .as_ref()
                            .is_none_or(|old| old.session.current != library.session.current)
                        {
                            // A new library selection must never be paired with
                            // the previous recording's cached playback snapshot.
                            cache.playback = None;
                        }
                        cache.library = Some(library);
                        cache.library_dirty = true;
                    }
                    Event::Playback(playback) => {
                        cache.playback = Some(playback);
                        cache.playback_dirty = true;
                    }
                    Event::Volume(volume, muted) => {
                        if let Some(library) = &mut cache.library {
                            library.session.volume = volume;
                            library.session.muted = muted;
                            cache.volume_dirty = true;
                        }
                    }
                    Event::Settings(settings) => {
                        cache.settings = Some(settings);
                        cache.settings_dirty = true;
                    }
                    Event::Drafts(drafts, issues) => {
                        cache.drafts = Some((drafts, issues));
                        cache.drafts_dirty = true;
                    }
                    event => {
                        // Consuming a status in one View must not erase it for
                        // the next Activity. Keep live events in order as well.
                        match &event {
                            Event::Scanning(active, message) => {
                                cache.scanning = (*active, message.clone());
                            }
                            Event::SourceUpdating(active) => cache.source_updating = *active,
                            Event::Hidden(hidden) => cache.hidden = *hidden,
                            Event::Show => cache.hidden = false,
                            _ => {}
                        }
                        cache.pending.push(event);
                    }
                }
                if let Some(tx) = &cache.subscriber {
                    let _ = tx.try_send(Event::AndroidRefresh);
                }
            }
        })?;
    let _ = RUNTIME.set(Runtime {
        tx: handle.tx.clone(),
        mailbox,
        _owner: Mutex::new(handle),
    });
    Ok(())
}
pub fn send(command: Command) {
    if let Some(runtime) = RUNTIME.get() {
        let _ = runtime.tx.send(command);
    }
}
pub fn attach(_options: Options) -> Result<Handle> {
    let runtime = RUNTIME
        .get()
        .ok_or_else(|| anyhow::anyhow!("Android runtime is not initialized"))?;
    let (tx, rx) = async_channel::bounded(1);
    let mut cache = runtime.mailbox.lock().unwrap();
    cache.library_dirty = true;
    cache.playback_dirty = true;
    cache.settings_dirty = true;
    cache.drafts_dirty = true;
    cache.status_dirty = true;
    let _ = tx.try_send(Event::AndroidRefresh);
    cache.subscriber = Some(tx);
    Ok(Handle::client(runtime.tx.clone(), rx))
}
pub fn detach() {
    if let Some(runtime) = RUNTIME.get() {
        runtime.mailbox.lock().unwrap().subscriber = None;
    }
    send(Command::Hidden(true));
}
pub struct UiEvents {
    pub snapshots: Vec<Event>,
    pub pending: Vec<Event>,
}

pub fn take_events() -> UiEvents {
    let mut cache = RUNTIME.get().unwrap().mailbox.lock().unwrap();
    let mut events = Vec::new();
    if std::mem::take(&mut cache.settings_dirty)
        && let Some(settings) = &cache.settings
    {
        events.push(Event::Settings(settings.clone()));
    }
    if std::mem::take(&mut cache.library_dirty)
        && let Some(library) = &cache.library
    {
        events.push(Event::Library(library.clone()));
    }
    if std::mem::take(&mut cache.playback_dirty)
        && let Some(playback) = &cache.playback
    {
        events.push(Event::Playback(playback.clone()));
    }
    if std::mem::take(&mut cache.volume_dirty)
        && let Some(library) = &cache.library
    {
        events.push(Event::Volume(library.session.volume, library.session.muted));
    }
    if std::mem::take(&mut cache.drafts_dirty)
        && let Some((drafts, issues)) = &cache.drafts
    {
        events.push(Event::Drafts(drafts.clone(), issues.clone()));
    }
    if std::mem::take(&mut cache.status_dirty) {
        events.push(Event::Scanning(cache.scanning.0, cache.scanning.1.clone()));
        events.push(Event::SourceUpdating(cache.source_updating));
        events.push(Event::Hidden(cache.hidden));
    }
    UiEvents {
        snapshots: events,
        pending: cache.pending.take(),
    }
}
