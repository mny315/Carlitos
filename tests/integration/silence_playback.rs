#![cfg(target_os = "linux")]
use carlitos::{
    library::file_uri,
    player::{Event, EventKind, Phase, Player},
};
use std::{
    io::Write,
    time::{Duration, Instant},
};

fn fixture(path: &std::path::Path, sections: &[(u32, bool)]) -> anyhow::Result<()> {
    let samples: u32 = sections.iter().map(|(ms, _)| ms * 8).sum();
    let mut file = std::fs::File::create(path)?;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + samples * 4).to_le_bytes())?;
    file.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x02\0")?;
    file.write_all(&8000u32.to_le_bytes())?;
    file.write_all(&32000u32.to_le_bytes())?;
    file.write_all(b"\x04\0\x10\0data")?;
    file.write_all(&(samples * 4).to_le_bytes())?;
    for &(ms, sound) in sections {
        for i in 0..ms * 8 {
            let value = if sound {
                ((i as f64 * 440. * std::f64::consts::TAU / 8000.).sin() * 6000.) as i16
            } else {
                0
            };
            // Speech in the right channel only must never be mistaken for silence.
            file.write_all(&0i16.to_le_bytes())?;
            file.write_all(&value.to_le_bytes())?;
        }
    }
    Ok(())
}
fn pump(
    context: &gst::glib::MainContext,
    player: &mut Player,
    events: &async_channel::Receiver<Event>,
) -> anyhow::Result<()> {
    while context.pending() {
        context.iteration(false);
    }
    while let Ok(event) = events.try_recv() {
        if let EventKind::Error(e) = &event.kind {
            anyhow::bail!("{e}");
        }
        player.handle(&event)?;
    }
    player.poll();
    Ok(())
}
fn settle(
    context: &gst::glib::MainContext,
    player: &mut Player,
    events: &async_channel::Receiver<Event>,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        pump(context, player, events)?;
        if player.phase == Phase::Ready {
            return Ok(());
        }
        anyhow::ensure!(
            Instant::now() < deadline,
            "timed out settling {:?}",
            player.phase
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn advance(
    context: &gst::glib::MainContext,
    player: &mut Player,
    events: &async_channel::Receiver<Event>,
    ms: u64,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < deadline {
        pump(context, player, events)?;
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
#[test]
fn silence_skip_preserves_source_time_seeks_speed_pause_and_eos() -> anyhow::Result<()> {
    let context = gst::glib::MainContext::new();
    context.with_thread_default(|| -> anyhow::Result<()> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("stereo.wav");
        fixture(
            &path,
            &[(1000, true), (3000, false), (1000, true), (2000, false)],
        )?;
        let uri = file_uri(&path)?;
        let (tx, rx) = async_channel::unbounded();
        let mut player = Player::new(tx, true)?;
        player.load(&uri, 1500, false)?;
        settle(&context, &mut player, &rx)?;
        assert_eq!(player.duration, Some(7000));
        player.set_playing(true)?;
        advance(&context, &mut player, &rx, 800)?;
        assert!(
            (2200..=2500).contains(&player.position),
            "off: {}",
            player.position
        );
        player.set_playing(false)?;
        player.seek(1500)?;
        player.set_skip_silence(true)?;
        settle(&context, &mut player, &rx)?;
        assert!(!player.playing);
        assert!(player.position.abs_diff(1500) < 100);
        advance(&context, &mut player, &rx, 150)?;
        assert!(
            player.position.abs_diff(1500) < 100,
            "pause moved: {}",
            player.position
        );
        for rate in [1.0, 2.0, 0.5] {
            player.seek(1500)?;
            player.set_rate(rate)?;
            settle(&context, &mut player, &rx)?;
            player.set_playing(true)?;
            advance(&context, &mut player, &rx, (850. / rate) as u64)?;
            assert!(
                player.take_silence_skip(),
                "source jump was not reported to MPRIS/checkpoints"
            );
            assert!(
                !player.take_silence_skip(),
                "source jump was reported twice"
            );
            assert!(
                (4000..=4600).contains(&player.position),
                "rate {rate}: expected original file position after silence, got {}",
                player.position
            );
            let position = player.position;
            player.set_skip_silence(false)?;
            settle(&context, &mut player, &rx)?;
            assert!(player.playing, "toggle interrupted playback");
            assert!(
                player.position.abs_diff(position) < 150,
                "toggle moved source position"
            );
            player.set_playing(false)?;
            player.set_skip_silence(true)?;
            settle(&context, &mut player, &rx)?;
            assert!(!player.playing);
        }
        player.set_rate(1.)?;
        player.load(&uri, 4500, false)?;
        settle(&context, &mut player, &rx)?;
        assert!(player.skip_silence());
        assert!(
            player.position.abs_diff(4500) < 100,
            "resume used compacted time"
        );
        player.set_playing(true)?;
        let started = Instant::now();
        while !player.ended && started.elapsed() < Duration::from_secs(3) {
            pump(&context, &mut player, &rx)?;
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(player.ended, "trailing silence did not finish");
        assert!(
            started.elapsed() < Duration::from_millis(1800),
            "trailing silence wasn't skipped"
        );
        assert_eq!(player.position, 7000);
        let silent = dir.path().join("silent.wav");
        fixture(&silent, &[(10_000, false)])?;
        player.load(&file_uri(&silent)?, 0, true)?;
        settle(&context, &mut player, &rx)?;
        let started = Instant::now();
        while !player.ended && started.elapsed() < Duration::from_secs(2) {
            pump(&context, &mut player, &rx)?;
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(player.ended, "silent file did not finish");
        assert_eq!(player.position, 10_000);
        Ok(())
    })??;
    Ok(())
}
