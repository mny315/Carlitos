#![cfg(target_os = "linux")]
use carlitos::{
    import::{self, ImportMode, ScanControl},
    library::*,
    player::{Event, Phase, Player},
    storage::Store,
};
use gst::glib;
use gst::prelude::*;
use std::time::{Duration, Instant};

fn settle(
    player: &mut Player,
    events: &async_channel::Receiver<Event>,
    timeout: Duration,
) -> anyhow::Result<()> {
    let start = Instant::now();
    let context = glib::MainContext::default();
    loop {
        while context.pending() {
            context.iteration(false);
        }
        while let Ok(event) = events.try_recv() {
            if let carlitos::player::EventKind::Error(e) = &event.kind {
                anyhow::bail!("{e}");
            }
            player.handle(&event)?;
        }
        if player.phase == Phase::Ready {
            return Ok(());
        }
        if start.elapsed() > timeout {
            anyhow::bail!("Player timed out: {:?}", player.phase);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
#[ignore = "requires generated fixtures: ./build.sh test media"]
fn codecs_seeks_import_resume_and_relocation() -> anyhow::Result<()> {
    let context = glib::MainContext::default();
    let _guard = context.acquire().unwrap();
    let root = std::path::PathBuf::from(std::env::var("CARLITOS_FIXTURES")?);
    if std::env::var_os("CARLITOS_REAL_AUDIO").is_some() {
        gst::init()?;
        let pipeline = gst::parse::launch(
            "filesrc name=source ! wavparse ! audioconvert ! audioresample ! volume mute=true ! pulsesink",
        )?
        .downcast::<gst::Pipeline>()
        .expect("audio output test pipeline");
        pipeline
            .by_name("source")
            .unwrap()
            .set_property("location", root.join("sample.wav").to_str().unwrap());
        pipeline.set_state(gst::State::Playing)?;
        let message = pipeline.bus().unwrap().timed_pop_filtered(
            gst::ClockTime::from_seconds(20),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        );
        pipeline.set_state(gst::State::Null)?;
        match message.as_ref().map(|m| m.view()) {
            Some(gst::MessageView::Eos(_)) => {
                println!("Real PulseAudio/PipeWire output passed (muted)")
            }
            Some(gst::MessageView::Error(e)) => anyhow::bail!("Audio output: {}", e.error()),
            _ => anyhow::bail!("Audio output timed out"),
        }
    }
    let (tx, rx) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    for filename in [
        "sample.wav",
        "sample.mp3",
        "sample.flac",
        "sample.aac",
        "sample.m4a",
        "sample.m4b",
        "he-aac-v1.m4b",
        "he-aac-v2.m4b",
    ] {
        let extension = filename;
        player.set_skip_silence(false)?;
        let uri = file_uri(&root.join(filename))?;
        player.load(&uri, 0, false)?;
        settle(&mut player, &rx, Duration::from_secs(10))?;
        assert!(player.seekable, "{extension} must seek");
        // Raw ADTS has no container duration; aacparse may only estimate it
        // after reading more frames. Container formats must expose it at load.
        if !filename.ends_with(".aac") || player.duration.is_some() {
            assert!(
                player
                    .duration
                    .is_some_and(|d| (11_000..=13_000).contains(&d)),
                "{extension}: {:?}",
                player.duration
            );
        }
        player.seek(7_000)?;
        settle(&mut player, &rx, Duration::from_secs(5))?;
        assert!(
            player.position.abs_diff(7_000) <= 1000,
            "{extension}: {}",
            player.position
        );
        assert!(!player.playing);
        player.seek(2_000)?;
        player.seek(9_000)?;
        player.seek(4_000)?;
        assert_eq!(player.seek_base(), 4_000);
        settle(&mut player, &rx, Duration::from_secs(5))?;
        assert!(
            player.position.abs_diff(4_000) <= 1000,
            "rapid seek {extension}: {}",
            player.position
        );
        player.load(&uri, 6_000, false)?;
        settle(&mut player, &rx, Duration::from_secs(5))?;
        assert!(
            player.position.abs_diff(6_000) <= 1000,
            "restore {extension}: {}",
            player.position
        );
        player.set_playing(true)?;
        let until = Instant::now() + Duration::from_millis(450);
        while Instant::now() < until {
            while context.pending() {
                context.iteration(false);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        player.poll();
        assert!(player.position > 6_100);
        player.set_playing(false)?;
        // Continuous audio must retain its timing at every speed with silence skip enabled.
        player.set_skip_silence(true)?;
        settle(&mut player, &rx, Duration::from_secs(5))?;
        for rate in [0.5, 2.0, 3.0] {
            player.seek(2_000)?;
            player.set_rate(rate)?;
            settle(&mut player, &rx, Duration::from_secs(5))?;
            assert!(!player.playing, "changing speed must preserve pause");
            assert!(
                player.position.abs_diff(2_000) < 150,
                "{extension} at {rate}×: speed changed position to {}",
                player.position
            );
            player.set_playing(true)?;
            let started = Instant::now();
            while started.elapsed() < Duration::from_millis(600) {
                while context.pending() {
                    context.iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            player.poll();
            let advanced = player.position.saturating_sub(2_000) as f64;
            let expected = started.elapsed().as_secs_f64() * 1000. * rate;
            assert!(
                (advanced - expected).abs() < 250.,
                "{extension} at {rate}×: advanced {advanced}, expected {expected}"
            );
            // Changing speed while playing preserves both position and intent.
            let before = player.position;
            player.set_rate(1.25)?;
            settle(&mut player, &rx, Duration::from_secs(5))?;
            assert!(player.playing);
            assert!(player.position.abs_diff(before) < 250);
            player.set_playing(false)?;
        }
        player.seek(3_000)?;
        player.set_rate(1.5)?;
        player.seek(4_000)?;
        player.set_rate(2.0)?;
        settle(&mut player, &rx, Duration::from_secs(5))?;
        assert!(player.position.abs_diff(4_000) < 150);
        assert_eq!(player.rate(), 2.0);
        player.load(&uri, 0, false)?;
        settle(&mut player, &rx, Duration::from_secs(5))?;
        assert_eq!(player.rate(), 2.0, "speed must survive a part change");
        assert!(!player.playing);
        player.set_rate(1.0)?;
        settle(&mut player, &rx, Duration::from_secs(5))?;
        player.seek(11_500)?;
        settle(&mut player, &rx, Duration::from_secs(5))?;
        player.set_playing(true)?;
        let started = Instant::now();
        let eos = loop {
            while context.pending() {
                context.iteration(false);
            }
            if let Ok(event) = rx.try_recv() {
                player.handle(&event)?;
                if matches!(event.kind, carlitos::player::EventKind::Eos(_)) {
                    assert!(player.ended, "{extension}: current EOS must be accepted");
                    break event;
                }
            }
            anyhow::ensure!(
                started.elapsed() < Duration::from_secs(5),
                "{extension}: EOS timeout"
            );
            std::thread::sleep(Duration::from_millis(5));
        };
        player.set_playing(false)?;
        player.seek(4_000)?;
        // Reproduce delivery of an old EOS on both sides of seek completion.
        player.handle(&eos)?;
        assert!(!player.ended, "{extension}: stale EOS during seek");
        settle(&mut player, &rx, Duration::from_secs(5))?;
        player.handle(&eos)?;
        assert!(!player.ended, "{extension}: stale EOS after seek");
        assert!(player.position.abs_diff(4_000) < 150);
        println!(
            "{extension}: decode, seek, resume, playback speeds and EOS ordering passed ({} ms)",
            player.position
        );
    }
    player.stop();
    // Missing image/parser plugins must not silently discard embedded covers
    // or the author/title while reducing the packaged audio runtime.
    for filename in ["covered.mp3", "covered.flac", "covered.m4a"] {
        let scan = import::scan(
            vec![root.join(filename)],
            ImportMode::Book,
            ScanControl::default(),
        )?;
        assert!(scan.issues.is_empty(), "{filename}: {:?}", scan.issues);
        let media = &scan.drafts[0].files[0];
        assert_eq!(media.artist, "Carlitos", "{filename}");
        assert_eq!(media.album, "Проверка", "{filename}");
        assert!(
            media
                .cover
                .as_ref()
                .is_some_and(|p| std::path::Path::new(p).is_file()),
            "{filename}: embedded cover missing"
        );
    }
    let m4b = import::scan(
        vec![root.join("chapters.m4b"), root.join("sample.m4b")],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    let files: Vec<_> = m4b.drafts.iter().flat_map(|d| &d.files).collect();
    assert_eq!(files.len(), 2);
    let chaptered = files
        .iter()
        .find(|f| f.uri.ends_with("chapters.m4b"))
        .unwrap();
    assert_eq!(chaptered.chapters.len(), 2);
    assert_eq!(chaptered.chapters[1].start, 5_000);
    assert!(
        files
            .iter()
            .find(|f| f.uri.ends_with("sample.m4b"))
            .unwrap()
            .chapters
            .is_empty()
    );
    println!(
        "M4B: {} internal chapters detected; no-TOC fallback is a single playable file",
        chaptered.chapters.len()
    );
    let scan = import::scan(
        vec![root.join("Книга с пробелом"), root.join("Вторая книга")],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    assert_eq!(scan.drafts.len(), 2);
    let names: Vec<_> = scan.drafts[0]
        .files
        .iter()
        .map(|m| m.relative.as_str())
        .collect();
    assert_eq!(names, ["Disc 1/1.wav", "Disc 1/2.wav", "Disc 1/10.wav"]);
    assert!(scan.issues.iter().any(|s| s.contains("символическая")));
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("db");
    let mut store = Store::open(&path)?;
    let mut lib = store.import(scan.drafts.clone())?;
    lib.session.current = Some(Target::Book(lib.parts[0].id));
    lib.update_progress(5000, false);
    let expected = lib.progress[0].clone();
    store.save(&lib.session, &lib.progress)?;
    store.import(scan.drafts)?;
    drop(store);
    let mut store = Store::open(&path)?;
    let lib = store.load()?;
    assert_eq!(lib.media.len(), 4);
    assert_eq!(lib.progress[0].position, 5000);
    assert_eq!(lib.progress[0].part_id, expected.part_id);
    let relocated = temp.path().join("new book");
    std::fs::create_dir_all(relocated.join("Disc 1"))?;
    for part in [1, 2, 10] {
        std::fs::copy(
            root.join(format!("Книга с пробелом/Disc 1/{part}.wav")),
            relocated.join(format!("Disc 1/{part}.wav")),
        )?;
    }
    let source = lib
        .sources
        .iter()
        .find(|s| {
            !s.uri.contains("Disc")
                && local_path(&s.uri).is_some_and(|p| p.ends_with("Книга с пробелом"))
        })
        .unwrap();
    let moved = store.relocate(source.id, &relocated)?;
    assert_eq!(moved.progress[0].part_id, expected.part_id);
    assert_eq!(moved.progress[0].position, 5000);
    assert!(
        moved
            .media
            .iter()
            .filter(|f| f.source_id == source.id)
            .all(|m| local_path(&m.uri).unwrap().starts_with(&relocated))
    );
    let scan = import::scan(
        vec![root.join("broken.mp3"), root.join("sample.wav")],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    assert_eq!(scan.drafts.len(), 1);
    assert!(scan.issues.iter().any(|s| s.contains("broken.mp3")));
    let cancelled = ScanControl::default();
    cancelled.cancel();
    assert!(import::scan(vec![root], ImportMode::Books, cancelled).is_err());
    println!(
        "Import, natural ordering, corrupt file isolation, cancellation, dedup, progress and relocation passed"
    );
    Ok(())
}
