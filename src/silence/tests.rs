use super::*;
#[test]
fn malformed_timestamps_cannot_overflow_the_streaming_callback() {
    gst::init().unwrap();
    let mut timeline = Timeline {
        enabled: true,
        quiet: KEEP_NS,
        segment: Some(gst::FormattedSegment::new()),
        ..Default::default()
    };
    let mut audio = buffer(0, &[0.0]);
    audio.make_mut().set_pts(gst::ClockTime::MAX);
    assert_eq!(timeline.process(&mut audio), gst::PadProbeReturn::Ok);
    timeline.jumps.push_back((0, 100));
    let silence = Silence(Arc::new(Mutex::new(timeline)));
    assert_eq!(
        silence.source_position(gst::ClockTime::MAX),
        gst::ClockTime::MAX
    );
}
fn buffer(pts_ms: u64, samples: &[f32]) -> gst::Buffer {
    let bytes: Vec<_> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut buffer = gst::Buffer::from_mut_slice(bytes);
    let b = buffer.get_mut().unwrap();
    b.set_pts(gst::ClockTime::from_mseconds(pts_ms));
    b.set_duration(gst::ClockTime::from_mseconds(100));
    buffer
}
#[test]
fn preserves_speech_in_either_channel_and_short_pauses() {
    gst::init().unwrap();
    let mut timeline = Timeline {
        enabled: true,
        ..Default::default()
    };
    timeline.reset(Some(gst::FormattedSegment::new()));
    for i in 0..20 {
        let mut audio = buffer(i * 100, &[0.0, 0.01]);
        assert_eq!(timeline.process(&mut audio), gst::PadProbeReturn::Ok);
        assert_eq!(audio.pts().unwrap().mseconds(), i * 100);
    }
    for i in 20..25 {
        assert_eq!(
            timeline.process(&mut buffer(i * 100, &[0.0, 0.0])),
            gst::PadProbeReturn::Ok
        );
    }
    assert_eq!(
        timeline.process(&mut buffer(2500, &[0.01, 0.0])),
        gst::PadProbeReturn::Ok
    );
    assert_eq!(timeline.skipped, 0);
}
#[test]
fn compacts_long_silence_maps_file_time_and_resets_on_seek() {
    gst::init().unwrap();
    let mut timeline = Timeline {
        enabled: true,
        ..Default::default()
    };
    timeline.reset(Some(gst::FormattedSegment::new()));
    timeline.process(&mut buffer(0, &[0.1]));
    for i in 1..11 {
        let result = timeline.process(&mut buffer(i * 100, &[0.0]));
        assert_eq!(
            result,
            if i <= 5 {
                gst::PadProbeReturn::Ok
            } else {
                gst::PadProbeReturn::Drop
            }
        );
    }
    let mut speech = buffer(1100, &[0.2]);
    let original = speech.map_readable().unwrap().to_vec();
    assert_eq!(timeline.process(&mut speech), gst::PadProbeReturn::Ok);
    assert_eq!(speech.pts().unwrap().mseconds(), 600);
    assert_eq!(speech.map_readable().unwrap().as_slice(), original);
    assert_eq!(timeline.source_position(550_000_000), 550_000_000);
    assert!(!timeline.discontinuity);
    assert_eq!(timeline.source_position(650_000_000), 1_150_000_000);
    assert!(timeline.discontinuity);
    timeline.reset(Some(gst::FormattedSegment::new()));
    assert_eq!(timeline.source_position(100_000_000), 100_000_000);
    assert_eq!(timeline.skipped, 0);
}
#[test]
fn long_silence_uses_one_time_mapping_and_off_passes_through() {
    gst::init().unwrap();
    let mut timeline = Timeline {
        enabled: true,
        ..Default::default()
    };
    timeline.reset(Some(gst::FormattedSegment::new()));
    for i in 0..10_000 {
        timeline.process(&mut buffer(i * 100, &[0.0]));
    }
    assert_eq!(timeline.jumps.len(), 1);
    timeline.enabled = false;
    timeline.reset(Some(gst::FormattedSegment::new()));
    for i in 0..20 {
        assert_eq!(
            timeline.process(&mut buffer(i * 100, &[0.0])),
            gst::PadProbeReturn::Ok
        );
    }
    assert_eq!(timeline.skipped, 0);
}
