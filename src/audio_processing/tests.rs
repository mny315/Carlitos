use super::*;
#[test]
fn pcm_rejects_partial_samples_and_channel_frames() {
    for channels in [1, 2, 8] {
        let frame = vec![0; usize::from(channels) * 2];
        for length in 1..frame.len() {
            assert!(
                decode_pcm16(&frame[..length], channels).is_err(),
                "accepted {length} bytes for {channels} channels"
            );
        }
        assert_eq!(
            decode_pcm16(&frame, channels).unwrap(),
            vec![0; channels as usize]
        );
        assert!(decode_pcm16(&[], channels).unwrap().is_empty());
    }
    assert!(decode_pcm16(&[], 0).is_err());
}

#[test]
fn pcm_preserves_signed_samples_and_channel_order_at_any_alignment() {
    let samples = [i16::MIN, -1, 0, i16::MAX, 1234, -2345];
    let mut bytes = vec![37];
    bytes.extend(samples.iter().flat_map(|value| value.to_le_bytes()));
    for channels in [1, 2, 3, 6] {
        assert_eq!(decode_pcm16(&bytes[1..], channels).unwrap(), samples);
    }
}
#[test]
fn quiet_compaction_preserves_channels_and_maps_progress() {
    let mut t = Timeline::default();
    let mut pcm = vec![0; 2000]; // one second, stereo, 1 kHz
    pcm.extend([0, 4000]);
    let out = t.filter(&pcm, 2, 1000, true);
    assert_eq!(out.len(), 1002);
    assert_eq!(&out[1000..], &[0, 4000]);
    assert_eq!(t.source_frame(499), 499);
    assert!(!t.discontinuity);
    assert_eq!(t.source_frame(500), 1000);
    assert!(t.discontinuity);
}
#[test]
fn disabled_filter_and_short_pauses_preserve_pcm() {
    for enabled in [false, true] {
        let mut t = Timeline::default();
        let pcm = vec![0; 400];
        assert_eq!(t.filter(&pcm, 1, 1000, enabled), pcm);
        assert_eq!(t.source_frame(400), 400);
    }
    let mut t = Timeline::default();
    let pcm = vec![i16::MIN; 2000];
    assert_eq!(t.filter(&pcm, 2, 1000, true), pcm);
}

#[test]
fn disabling_the_filter_preserves_time_and_resets_the_quiet_run() {
    let mut t = Timeline::default();
    assert_eq!(t.filter(&[0; 1000], 1, 1000, true).len(), 500);
    assert_eq!(t.filter(&[17; 10], 1, 1000, false), [17; 10]);
    assert_eq!(t.source_frame(509), 1009);
    // The next quiet interval keeps its own first half-second.
    assert_eq!(t.filter(&[0; 501], 1, 1000, true).len(), 500);
    assert_eq!(t.source_frame(1009), 1509);
    assert_eq!(t.source_frame(1010), 1511);
}
