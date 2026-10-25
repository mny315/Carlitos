use super::*;
use std::io::Write;
use std::time::{Duration, Instant};

#[test]
fn decoded_ahead_bands_wait_for_presentation_and_expire_after_output_stalls() {
    let mut samples = Samples::default();
    samples.push(500_000_000, [0.8, 0.4, 0.2, 0.1]);
    samples.push(580_000_000, [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(samples.read(0), [0.; 4]);
    assert_eq!(samples.read(499_999_999), [0.; 4]);
    assert_eq!(samples.read(500_000_000), [0.8, 0.4, 0.2, 0.1]);
    assert_eq!(samples.read(579_999_999), [0.8, 0.4, 0.2, 0.1]);
    assert_eq!(samples.read(580_000_000), [0.1, 0.2, 0.3, 0.4]);
    assert_eq!(samples.read(780_000_001), [0.; 4]);
}

#[test]
fn sampling_uses_presentation_time_and_keeps_a_bounded_queue() {
    let mut samples = Samples::default();
    for i in 0..CAPACITY {
        let time = i as u64 * INTERVAL;
        assert!(samples.ready(time));
        samples.push(time, [0.4; 4]);
        assert!(!samples.ready(time + INTERVAL - 1));
    }
    assert!(!samples.ready(CAPACITY as u64 * INTERVAL));
    assert_eq!(samples.read(0), [0.4; 4]);
    assert!(samples.ready(CAPACITY as u64 * INTERVAL));
}

fn wait_until(mut check: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !check() {
        assert!(Instant::now() < deadline, "audio test timed out");
        std::thread::sleep(Duration::from_millis(2));
    }
}

#[test]
fn output_clock_prevents_queued_tone_from_appearing_before_the_sink_plays_it() -> anyhow::Result<()>
{
    gst::init()?;
    // Use the same WAV decoder as ordinary playback; the runtime deliberately
    // excludes GStreamer's synthetic audio-source plugin.
    let mut audio = tempfile::NamedTempFile::new()?;
    let frames = 72_000u32;
    audio.write_all(b"RIFF")?;
    audio.write_all(&(36 + frames * 2).to_le_bytes())?;
    audio.write_all(b"WAVEfmt \x10\0\0\0\x01\0\x01\0")?;
    audio.write_all(&48_000u32.to_le_bytes())?;
    audio.write_all(&96_000u32.to_le_bytes())?;
    audio.write_all(b"\x02\0\x10\0data")?;
    audio.write_all(&(frames * 2).to_le_bytes())?;
    for frame in 0..frames {
        let sample = if frame < 24_000 {
            0
        } else {
            ((f64::from(frame) * 440. * std::f64::consts::TAU / 48_000.).sin() * 6000.) as i16
        };
        audio.write_all(&sample.to_le_bytes())?;
    }
    audio.flush()?;
    let pipeline = gst::parse::launch(
        "filesrc name=input ! wavparse ! audioconvert \
         ! audio/x-raw,format=F32LE,rate=48000,channels=1 \
         ! identity name=voice \
         ! queue max-size-buffers=0 max-size-bytes=0 max-size-time=2000000000 \
         ! fakesink name=output sync=true signal-handoffs=true",
    )?
    .downcast::<gst::Pipeline>()
    .unwrap();
    pipeline
        .by_name("input")
        .unwrap()
        .set_property("location", audio.path().to_str().unwrap());
    struct Stop(gst::Pipeline);
    impl Drop for Stop {
        fn drop(&mut self) {
            let _ = self.0.set_state(gst::State::Null);
        }
    }
    let _stop = Stop(pipeline.clone());
    let voice = pipeline.by_name("voice").unwrap();
    let output = Output::attach(
        pipeline.upcast_ref(),
        &voice.static_pad("src").unwrap(),
        true,
    );
    let sounded = Arc::new(AtomicBool::new(false));
    let heard = sounded.clone();
    pipeline
        .by_name("output")
        .unwrap()
        .connect("handoff", false, move |args| {
            let buffer = args[1].get::<gst::Buffer>().unwrap();
            let map = buffer.map_readable().unwrap();
            if map
                .as_chunks::<4>()
                .0
                .iter()
                .any(|bytes| f32::from_le_bytes(*bytes).abs() > 0.01)
            {
                heard.store(true, Ordering::Relaxed);
            }
            None
        });
    pipeline.set_state(gst::State::Paused)?;
    pipeline.state(gst::ClockTime::from_seconds(2)).0?;
    wait_until(|| {
        output
            .timeline
            .lock()
            .unwrap()
            .samples
            .pending
            .iter()
            .any(|sample| sample.1.iter().any(|value| *value > 0.))
    });
    assert!(
        !sounded.load(Ordering::Relaxed),
        "tone must still be queued"
    );
    assert_eq!(output.read(), [0.; 4], "preroll must stay invisible");
    pipeline.set_state(gst::State::Playing)?;
    let mut visible = false;
    let started = Instant::now();
    while started.elapsed() < Duration::from_millis(900) {
        let levels = output.read();
        if levels.iter().any(|value| *value > 0.) {
            assert!(
                sounded.load(Ordering::Relaxed),
                "visualizer led audio output"
            );
            visible = true;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(visible, "visualizer must follow the audible tone");
    pipeline.set_state(gst::State::Paused)?;
    pipeline.state(gst::ClockTime::from_seconds(2)).0?;
    assert_eq!(output.read(), [0.; 4]);
    output.enable(false);
    assert_eq!(output.read(), [0.; 4]);
    output.enable(true);
    pipeline.set_state(gst::State::Playing)?;
    wait_until(|| output.read().iter().any(|value| *value > 0.));
    // A flushing seek must discard levels for the old position, and a
    // paused pipeline must never reveal its newly decoded preroll buffers.
    pipeline.set_state(gst::State::Paused)?;
    pipeline.state(gst::ClockTime::from_seconds(2)).0?;
    pipeline.seek_simple(
        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
        gst::ClockTime::ZERO,
    )?;
    pipeline.state(gst::ClockTime::from_seconds(2)).0?;
    assert_eq!(output.read(), [0.; 4]);
    Ok(())
}
