#![cfg(windows)]
use carlitos::{
    library::file_uri,
    player::{Event, EventKind, Phase, Player},
};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn fixture(path: &Path) -> anyhow::Result<()> {
    let rate = 16_000u32;
    let frames = rate * 4;
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + frames * 2).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(rate.to_le_bytes());
    wav.extend((rate * 2).to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((frames * 2).to_le_bytes());
    for i in 0..frames {
        let sample: i16 = if i >= rate && i < rate * 3 {
            0
        } else {
            ((f64::from(i) * 440. * std::f64::consts::TAU / f64::from(rate)).sin() * 8000.) as i16
        };
        wav.extend(sample.to_le_bytes());
    }
    std::fs::write(path, wav)?;
    Ok(())
}
fn advance(player: &mut Player, duration: Duration) {
    let start = Instant::now();
    while start.elapsed() < duration {
        player.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    player.poll();
}
#[test]
fn native_decode_seek_speed_silence_pause_and_end() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("Книга с пробелами.wav");
    fixture(&path)?;
    let (tx, _rx) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    player.load(&file_uri(&path)?, 500, false)?;
    assert_eq!(player.phase, Phase::Ready);
    assert_eq!(player.duration, Some(4000));
    assert_eq!(player.position, 500);
    player.set_rate(2.)?;
    player.set_playing(true)?;
    advance(&mut player, Duration::from_millis(200));
    assert!(
        (750..1100).contains(&player.position),
        "{}",
        player.position
    );
    player.set_playing(false)?;
    player.poll();
    let paused = player.position;
    advance(&mut player, Duration::from_millis(100));
    assert_eq!(player.position, paused);
    player.set_skip_silence(true)?;
    player.seek(0)?;
    player.set_playing(true)?;
    let start = Instant::now();
    let mut jumped = false;
    while !player.ended && start.elapsed() < Duration::from_secs(3) {
        advance(&mut player, Duration::from_millis(20));
        jumped |= player.take_silence_skip();
    }
    assert!(jumped && player.ended);
    assert_eq!(player.position, 4000);
    assert!(
        start.elapsed() < Duration::from_millis(1800),
        "silence must reduce playback time"
    );
    player.seek(200)?;
    assert!(!player.ended);
    assert_eq!(player.position, 200);
    player.stop();
    assert_eq!(player.phase, Phase::Empty);
    Ok(())
}

#[test]
fn decoder_errors_are_retryable_and_old_events_do_not_finish_a_seek() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let valid = dir.path().join("good.wav");
    let bad = dir.path().join("bad.mp3");
    fixture(&valid)?;
    std::fs::write(&bad, "not audio")?;
    let (tx, rx) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    assert!(player.load(&file_uri(&bad)?, 0, true).is_err());
    assert_eq!(player.phase, Phase::Error);
    player.load(&file_uri(&valid)?, 0, false)?;
    let old = rx.try_recv()?;
    player.seek(1000)?;
    assert!(!player.handle(&old)?);
    assert_eq!(player.position, 1000);
    assert!(player.set_rate(f64::NAN).is_err());
    assert_eq!(player.rate(), 1.);
    Ok(())
}

#[test]
fn a_queued_error_cannot_cancel_playback_after_a_successful_seek() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("retry.wav");
    fixture(&path)?;
    let (tx, rx) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    player.load(&file_uri(&path)?, 0, true)?;
    player.handle(&rx.try_recv()?)?;
    // A device/decoder error can be queued while the worker accepts a seek.
    let old_error = Event {
        generation: player.generation,
        kind: EventKind::Error("Previous audio output failed".into()),
    };
    player.seek(1000)?;
    assert!(!player.handle(&old_error)?);
    assert_eq!(player.phase, Phase::Ready);
    assert!(player.playing);
    assert!(player.handle(&rx.try_recv()?)?);
    advance(&mut player, Duration::from_millis(100));
    assert!(player.position > 1000);
    // Errors from the current playback must still stop it.
    let current_error = Event {
        generation: player.generation,
        kind: EventKind::Error("Current audio output failed".into()),
    };
    player.handle(&current_error)?;
    assert_eq!(player.phase, Phase::Error);
    assert!(!player.playing);
    Ok(())
}

#[test]
fn supported_codecs_decode_seek_and_reach_end() -> anyhow::Result<()> {
    use anyhow::Context;
    let dir = tempfile::tempdir()?;
    let (tx, rx) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    for (name, bytes) in [
        (
            "Моно.mp3",
            &include_bytes!("../fixtures/import-mono.mp3")[..],
        ),
        (
            "Стерео.mp3",
            &include_bytes!("../fixtures/import-stereo.mp3")[..],
        ),
        ("Книга.aac", &include_bytes!("../fixtures/import.aac")[..]),
        ("Книга.m4a", &include_bytes!("../fixtures/import.m4b")[..]),
        ("Книга.m4b", &include_bytes!("../fixtures/import.m4b")[..]),
        (
            "HE-AAC v1.m4b",
            &include_bytes!("../fixtures/import-he-aac-v1.m4b")[..],
        ),
        (
            "HE-AAC v2.m4b",
            &include_bytes!("../fixtures/import-he-aac-v2.m4b")[..],
        ),
        ("Книга.flac", &include_bytes!("../fixtures/import.flac")[..]),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, bytes)?;
        player
            .load(&file_uri(&path)?, 0, false)
            .with_context(|| format!("load {name}"))?;
        player.seek(400).with_context(|| format!("seek {name}"))?;
        player.set_rate(2.)?;
        player.set_playing(true)?;
        let started = Instant::now();
        while !player.ended && started.elapsed() < Duration::from_secs(3) {
            advance(&mut player, Duration::from_millis(20));
            while let Ok(event) = rx.try_recv() {
                if event.generation == player.generation {
                    if let EventKind::Error(message) = &event.kind {
                        anyhow::bail!("{name}: {message}");
                    }
                    player.handle(&event)?;
                }
            }
        }
        assert!(player.ended, "{name} did not finish: {:?}", player.phase);
        assert!(
            (900..=1300).contains(&player.position),
            "{name}: {}",
            player.position
        );
        player.seek(100)?;
        assert!(!player.ended, "{name}: seek after EOS");
        advance(&mut player, Duration::from_millis(100));
        assert!(player.position > 100, "{name}: did not restart after EOS");
    }
    Ok(())
}

#[test]
#[ignore = "requires a working Windows audio output device"]
fn native_xaudio2_output_advances_and_pauses() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("output.wav");
    fixture(&path)?;
    let (tx, _rx) = async_channel::unbounded();
    let mut player = Player::new(tx, false)?;
    player.volume(0.02, false);
    player.load(&file_uri(&path)?, 0, true)?;
    advance(&mut player, Duration::from_millis(400));
    assert!(player.position > 100, "audio device must consume samples");
    player.set_playing(false)?;
    player.poll();
    let paused = player.position;
    advance(&mut player, Duration::from_millis(100));
    assert_eq!(player.position, paused);
    Ok(())
}
