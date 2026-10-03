use super::*;

#[test]
fn failed_play_and_pause_transitions_leave_a_retryable_error() -> Result<()> {
    for playing in [true, false] {
        let (tx, _) = async_channel::unbounded();
        let mut player = Player::new(tx, true)?;
        let pipeline = gst::ElementFactory::make("fakesink")
            .property("async", false)
            .build()?;
        pipeline.set_state(if playing {
            gst::State::Paused
        } else {
            gst::State::Playing
        })?;
        pipeline.set_property_from_str(
            "state-error",
            if playing {
                "paused-to-playing"
            } else {
                "playing-to-paused"
            },
        );
        player.pipeline = Some(pipeline.clone());
        player.phase = Phase::Ready;
        player.playing = !playing;
        player.seekable = true;
        player.position = 4_000;
        assert!(player.set_playing(playing).is_err());
        assert_eq!(player.phase, Phase::Error);
        assert!(!player.playing && !player.seekable);
        assert_eq!(player.position, 4_000);
        pipeline.set_property_from_str("state-error", "none");
    }
    Ok(())
}

#[test]
fn eos_after_a_decode_error_cannot_mark_the_file_finished() -> Result<()> {
    let (tx, _) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    player.phase = Phase::Ready;
    player.position = 4_000;
    player.duration = Some(12_000);
    player.handle(&Event {
        generation: player.generation,
        kind: EventKind::Error("decode failed".into()),
    })?;
    player.handle(&Event {
        generation: player.generation,
        kind: EventKind::Eos(gst::Seqnum::next()),
    })?;
    assert!(!player.ended);
    assert_eq!(player.position, 4_000);
    assert_eq!(player.phase, Phase::Error);
    Ok(())
}

#[test]
fn rejected_queued_seek_leaves_a_retryable_error_instead_of_seeking_forever() -> Result<()> {
    let (tx, _) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    player.pipeline = Some(gst::ElementFactory::make("fakesink").build()?);
    player.phase = Phase::Seeking;
    player.playing = true;
    player.seekable = true;
    player.inflight_seek = Some(5_000);
    player.requested_seek = Some(7_000);
    assert!(
        player
            .handle(&Event {
                generation: player.generation,
                kind: EventKind::AsyncDone(gst::Seqnum::next())
            })
            .is_err()
    );
    assert_eq!(player.phase, Phase::Error);
    assert!(!player.playing && !player.seekable);
    Ok(())
}
#[test]
fn oversized_seek_and_nonfinite_volume_cannot_panic_the_audio_worker() -> Result<()> {
    let (tx, _) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    player.pipeline = Some(gst::ElementFactory::make("fakesink").build()?);
    player.phase = Phase::Ready;
    player.seekable = true;
    // A fakesink cannot seek, but the request must fail normally rather
    // than panic while converting milliseconds to nanoseconds.
    assert!(player.seek(u64::MAX).is_err());
    player.pipeline = None;
    player.volume(0.4, false);
    player.volume(f64::NAN, true);
    assert_eq!(player.volume, 0.4);
    assert!(player.muted);
    Ok(())
}
#[test]
fn eos_queued_before_seek_cannot_finish_the_new_position() -> Result<()> {
    let (tx, _) = async_channel::unbounded();
    let mut player = Player::new(tx, true)?;
    let old = Event {
        generation: player.generation,
        kind: EventKind::Eos(gst::Seqnum::next()),
    };
    player.seek_seqnum = Some(gst::Seqnum::next());
    player.position = 4_000;
    player.duration = Some(12_000);
    for phase in [Phase::Seeking, Phase::Ready] {
        player.phase = phase;
        player.handle(&old)?;
        assert!(!player.ended, "stale EOS during {phase:?}");
        assert_eq!(player.position, 4_000);
    }
    player.handle(&Event {
        generation: player.generation,
        kind: EventKind::Eos(gst::Seqnum::next()),
    })?;
    assert!(player.ended);
    assert_eq!(player.position, 12_000);
    Ok(())
}

#[test]
fn stale_events_cannot_change_new_selection() -> Result<()> {
    let (tx, _) = async_channel::unbounded();
    let mut p = Player::new(tx, true)?;
    p.generation = 7;
    p.phase = Phase::Loading;
    p.position = 37_000;
    p.handle(&Event {
        generation: 6,
        kind: EventKind::Error("old".into()),
    })?;
    assert_eq!(p.phase, Phase::Loading);
    assert_eq!(p.position, 37_000);
    p.seekable = true;
    p.handle(&Event {
        generation: 7,
        kind: EventKind::Error("current file".into()),
    })?;
    assert_eq!(p.phase, Phase::Error);
    assert!(!p.seekable && !p.playing);
    p.stop();
    assert_eq!(p.position, 0);
    Ok(())
}
