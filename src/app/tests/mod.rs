mod audio;
mod catalog;
mod persistence;
mod playback;
mod scanning;
mod volume;

use super::audio::AudioCommand;
use super::*;
use crate::import::{self, ImportMode};
use std::{path::Path, sync::mpsc, time::Duration};

fn fixture(root: &Path) -> Result<Library> {
    let folder = root.join("Книга с пробелами");
    std::fs::create_dir(&folder)?;
    for n in 1..=3 {
        let samples = 8_000 * 12u32;
        let size = samples * 2;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&(36 + size).to_le_bytes());
        bytes.extend_from_slice(b"WAVEfmt ");
        bytes.extend_from_slice(&16u32.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&8_000u32.to_le_bytes());
        bytes.extend_from_slice(&16_000u32.to_le_bytes());
        bytes.extend_from_slice(&2u16.to_le_bytes());
        bytes.extend_from_slice(&16u16.to_le_bytes());
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&size.to_le_bytes());
        bytes.resize(44 + size as usize, 0);
        std::fs::write(folder.join(format!("{n}.wav")), bytes)?;
    }
    let scan = import::scan(vec![folder], ImportMode::Book, ScanControl::default())?;
    Store::open(&root.join("db"))?.import(scan.drafts)
}

fn start(root: &Path) -> Handle {
    start_with_settings(root, Settings::default())
}

fn controller(root: &Path) -> Result<(Controller, mpsc::Receiver<Command>)> {
    let library = fixture(root)?;
    let (tx, rx) = mpsc::channel();
    let (events, _) = async_channel::unbounded();
    Ok((
        Controller {
            store: Some(Ok(Store::open(&root.join("db"))?)),
            library,
            audio: Audio::start(tx.clone(), true),
            playback: Playback::default(),
            requested_rate: 1.0,
            requested_skip_silence: false,
            tx,
            events,
            token: 0,
            scan_generation: 0,
            scan_control: ScanControl::default(),
            scan_source: None,
            scans: vec![],
            drafts: vec![],
            issues: vec![],
            checkpoint: Instant::now(),
            options: Options {
                database: root.join("db"),
                settings_path: root.join("settings.json"),
                settings: Settings::default(),
                settings_writable: true,
                demo: false,
                fake_audio: true,
            },
            quitting: false,
            pending_save: false,
            maintenance: false,
            maintenance_control: ScanControl::default(),
            tag_control: ScanControl::default(),
        },
        rx,
    ))
}

fn start_with_settings(root: &Path, settings: Settings) -> Handle {
    Handle::start(Options {
        database: root.join("db"),
        settings_path: root.join("settings.json"),
        settings,
        settings_writable: true,
        demo: false,
        fake_audio: true,
    })
    .expect("start test controller")
}

fn wait(app: &Handle, mut predicate: impl FnMut(&Event) -> bool) -> Result<Event> {
    let until = Instant::now() + Duration::from_secs(12);
    let mut recent = std::collections::VecDeque::new();
    loop {
        match app.events.try_recv() {
            Ok(event) => {
                recent.push_back(match &event {
                    Event::Playback(p) => format!("{p:?}"),
                    Event::Settings(s) => format!(
                        "settings: rate={}, skip={}",
                        s.playback_rate, s.skip_silence
                    ),
                    _ => "other event".into(),
                });
                if recent.len() > 8 {
                    recent.pop_front();
                }
                if let Event::Notice(message) = &event {
                    bail!("Unexpected notice: {message}");
                }
                if predicate(&event) {
                    return Ok(event);
                }
            }
            Err(async_channel::TryRecvError::Empty) => {}
            Err(e) => bail!("Controller disconnected: {e}"),
        }
        if Instant::now() >= until {
            bail!("Controller timed out: {recent:?}");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn ready(app: &Handle, position: u64, playing: bool) -> Result<Playback> {
    let Event::Playback(p) = wait(
        app,
        |event| matches!(event, Event::Playback(p) if p.phase == crate::player::Phase::Ready && p.position.abs_diff(position) < 600 && p.playing == playing),
    )?
    else {
        unreachable!()
    };
    Ok(p)
}

fn quit(app: &mut Handle) -> Result<()> {
    app.tx.send(Command::Quit)?;
    wait(app, |e| matches!(e, Event::Quit))?;
    app.join();
    Ok(())
}
