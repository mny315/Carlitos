mod instance;
mod mpris;
pub mod portal;
mod tray;

use crate::app::{Command, audio::Playback};
use carlitos::library::{Library, Target, file_uri};
pub use instance::Instance;
use mpris::{Media, MediaState, NAME, PATH, Root};
use std::{
    collections::HashMap,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    time::Duration,
};
use tray::Tray;
use zbus::{
    blocking::Connection,
    zvariant::{OwnedValue, Str, Value},
};

enum Update {
    Library(Box<Library>),
    Playback(Playback),
    Volume(f64),
    Quit,
}
pub struct Desktop {
    tx: Sender<Update>,
    thread: std::sync::Mutex<Option<std::thread::JoinHandle<()>>>,
    pub tray: Arc<AtomicBool>,
}
impl Desktop {
    pub fn start(instance: Option<(Connection, String)>, commands: Sender<Command>) -> Self {
        let (tx, rx) = mpsc::channel();
        let available = Arc::new(AtomicBool::new(false));
        let tray_available = available.clone();
        let startup = commands.clone();
        let thread = std::thread::Builder::new()
            .name("carlitos-desktop".into())
            .spawn(move || {
                let Some((connection, activation_name)) = instance else {
                    return;
                };
                let state = Arc::new(RwLock::new(MediaState {
                    rate: 1.0,
                    ..Default::default()
                }));
                let setup = (|| -> zbus::Result<()> {
                    connection.object_server().at(
                        PATH,
                        Root {
                            tx: commands.clone(),
                        },
                    )?;
                    connection.object_server().at(
                        PATH,
                        Media {
                            tx: commands.clone(),
                            state: state.clone(),
                        },
                    )?;
                    connection.request_name_with_flags(
                        activation_name.as_str(),
                        zbus::fdo::RequestNameFlags::DoNotQueue.into(),
                    )?;
                    // Separate libraries must neither replace each other's MPRIS
                    // service nor activate the wrong window on a second launch.
                    match connection.request_name_with_flags(
                        NAME,
                        zbus::fdo::RequestNameFlags::DoNotQueue.into(),
                    ) {
                        Ok(_) => {}
                        Err(zbus::Error::NameTaken) => {
                            connection.request_name_with_flags(
                                format!("{NAME}.instance{}", std::process::id()),
                                zbus::fdo::RequestNameFlags::DoNotQueue.into(),
                            )?;
                        }
                        Err(error) => return Err(error),
                    }
                    Ok(())
                })();
                if let Err(e) = setup {
                    let _ = commands.send(Command::Error(format!("MPRIS: {e}")));
                    return;
                }
                use ksni::blocking::TrayMethods;
                let tray = Tray {
                    tx: commands.clone(),
                    available: tray_available.clone(),
                }
                .assume_sni_available(true)
                .spawn()
                .ok();
                let prefix = format!("org.kde.StatusNotifierItem-{}-", std::process::id());
                let mut last_status = String::new();
                let mut last_meta = None;
                let mut last_volume = -1.;
                let mut last_rate = -1.;
                let mut last_caps = (false, false, false, false);
                let mut last_host = std::time::Instant::now() - Duration::from_secs(2);
                loop {
                    let update = rx.recv_timeout(Duration::from_secs(1));
                    let mut seeked = false;
                    match update {
                        Ok(Update::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Ok(Update::Library(library)) => {
                            let mut s = state.write().unwrap();
                            s.volume = if library.session.muted {
                                0.
                            } else {
                                library.session.volume
                            };
                            s.next = library.neighbour(true).is_some();
                            s.previous = library.neighbour(false).is_some();
                            if let Some(Target::Book(id)) = library.session.current
                                && let Some(part) = library.part(id)
                                && let Some(book) =
                                    library.books.iter().find(|b| b.id == part.book_id)
                            {
                                s.track = Some(id);
                                s.title = book.title.clone();
                                s.author = book.author.clone();
                                s.position = library.session.position;
                                s.art = book
                                    .cover
                                    .as_ref()
                                    .and_then(|p| file_uri(std::path::Path::new(p)).ok())
                                    .unwrap_or_default();
                                if let Some(file) = library.media(part.file_id) {
                                    s.uri = file.uri.clone();
                                    s.duration = file.duration;
                                }
                            } else {
                                s.track = None;
                                s.playing = false;
                                s.seekable = false;
                            }
                        }
                        Ok(Update::Volume(volume)) => state.write().unwrap().volume = volume,
                        Ok(Update::Playback(p)) => {
                            let mut s = state.write().unwrap();
                            s.position = p.position;
                            s.duration = p.duration;
                            s.playing = p.playing;
                            s.rate = p.rate;
                            s.seekable = p.seekable;
                            seeked = p.seek_done;
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                    }
                    let s = state.read().unwrap();
                    let status = s.status();
                    let meta = (
                        s.track,
                        s.title.clone(),
                        s.author.clone(),
                        s.uri.clone(),
                        s.art.clone(),
                        s.duration,
                    );
                    let caps = (s.next, s.previous, s.seekable, s.track.is_some());
                    let mut changed: HashMap<&str, OwnedValue> = HashMap::new();
                    if status != last_status {
                        changed.insert("PlaybackStatus", Str::from(status).into());
                        last_status = status.into();
                    }
                    if last_meta.as_ref() != Some(&meta) {
                        changed.insert("Metadata", Value::from(s.metadata()).try_into().unwrap());
                        last_meta = Some(meta);
                    }
                    if s.volume != last_volume {
                        changed.insert("Volume", s.volume.into());
                        last_volume = s.volume;
                    }
                    if s.rate != last_rate {
                        changed.insert("Rate", s.rate.into());
                        last_rate = s.rate;
                    }
                    if caps != last_caps {
                        changed.insert("CanGoNext", caps.0.into());
                        changed.insert("CanGoPrevious", caps.1.into());
                        changed.insert("CanSeek", caps.2.into());
                        changed.insert("CanPlay", caps.3.into());
                        changed.insert("CanPause", caps.3.into());
                        last_caps = caps;
                    }
                    if !changed.is_empty() {
                        let _ = connection.emit_signal(
                            None::<&str>,
                            PATH,
                            "org.freedesktop.DBus.Properties",
                            "PropertiesChanged",
                            &(
                                "org.mpris.MediaPlayer2.Player",
                                changed,
                                Vec::<String>::new(),
                            ),
                        );
                    }
                    if seeked {
                        let _ = connection.emit_signal(
                            None::<&str>,
                            PATH,
                            "org.mpris.MediaPlayer2.Player",
                            "Seeked",
                            &((s.position.min(i64::MAX as u64 / 1000) * 1000) as i64,),
                        );
                    }
                    drop(s);
                    if last_host.elapsed() >= Duration::from_secs(1) {
                        last_host = std::time::Instant::now();
                        let host = (|| -> zbus::Result<bool> {
                            let p = zbus::blocking::Proxy::new(
                                &connection,
                                "org.kde.StatusNotifierWatcher",
                                "/StatusNotifierWatcher",
                                "org.kde.StatusNotifierWatcher",
                            )?;
                            Ok(p.get_property::<bool>("IsStatusNotifierHostRegistered")?
                                && p.get_property::<Vec<String>>("RegisteredStatusNotifierItems")?
                                    .iter()
                                    .any(|item| item.starts_with(&prefix)))
                        })()
                        .unwrap_or(false)
                            && tray.is_some();
                        if tray_available.swap(host, Ordering::SeqCst) && !host {
                            let _ = commands.send(Command::Show);
                        }
                    }
                }
                if let Some(tray) = tray {
                    tray.shutdown();
                }
            });
        let thread = match thread {
            Ok(thread) => Some(thread),
            Err(error) => {
                let _ = startup.send(Command::Error(format!(
                    "{}: {error}",
                    crate::app::text(
                        "Не удалось запустить интеграцию с рабочим столом",
                        "Could not start desktop integration",
                    )
                )));
                None
            }
        };
        Self {
            tx,
            thread: std::sync::Mutex::new(thread),
            tray: available,
        }
    }
    pub fn library(&self, library: &Library) {
        let _ = self.tx.send(Update::Library(Box::new(library.clone())));
    }
    pub fn playback(&self, playback: &Playback) {
        let _ = self.tx.send(Update::Playback(playback.clone()));
    }
    pub fn volume(&self, volume: f64, muted: bool) {
        let _ = self
            .tx
            .send(Update::Volume(if muted { 0. } else { volume }));
    }
    pub fn stop(&self) {
        let _ = self.tx.send(Update::Quit);
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        self.stop();
    }
}
