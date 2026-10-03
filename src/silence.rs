//! Compact quiet PCM buffers before scaletempo, keeping a map back to file time.
//! Never mix channels or edit the source file. All channels must be quiet.
use anyhow::Result;
use gst::prelude::*;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

const KEEP_NS: u64 = 500_000_000;
const PEAK: f32 = 0.001_778_279_4; // -55 dBFS; conservative peak, not average loudness.

#[derive(Default)]
struct Timeline {
    enabled: bool,
    segment: Option<gst::FormattedSegment<gst::ClockTime>>,
    quiet: u64,
    skipped: u64,
    end: Option<u64>,
    // (compacted stream time, cumulative removed source time).
    jumps: VecDeque<(u64, u64)>,
    reported_skip: u64,
    discontinuity: bool,
}
impl Timeline {
    fn reset(&mut self, segment: Option<gst::FormattedSegment<gst::ClockTime>>) {
        *self = Self {
            enabled: self.enabled,
            segment,
            ..Self::default()
        };
    }
    fn source_position(&mut self, position: u64) -> u64 {
        // Keep the last applicable jump and the small amount decoded ahead.
        while self.jumps.get(1).is_some_and(|jump| jump.0 <= position) {
            self.jumps.pop_front();
        }
        let skipped = self
            .jumps
            .front()
            .filter(|j| j.0 <= position)
            .map_or(0, |j| j.1);
        self.discontinuity |= skipped != self.reported_skip;
        self.reported_skip = skipped;
        position.saturating_add(skipped)
    }
    fn process(&mut self, buffer: &mut gst::Buffer) -> gst::PadProbeReturn {
        let Some(pts) = buffer.pts() else {
            return gst::PadProbeReturn::Ok;
        };
        let Some(segment) = self.segment.as_ref() else {
            return gst::PadProbeReturn::Ok;
        };
        if !self.enabled || segment.rate() <= 0.0 {
            return gst::PadProbeReturn::Ok;
        }
        let quiet = buffer.map_readable().is_ok_and(|map| {
            !map.is_empty()
                && map.as_chunks::<4>().1.is_empty()
                && map.as_chunks::<4>().0.iter().all(|bytes| {
                    let sample = f32::from_le_bytes(*bytes);
                    sample.is_finite() && sample.abs() < PEAK
                })
        });
        let duration = buffer.duration().map_or(0, |d| d.nseconds());
        if self
            .end
            .is_some_and(|end| end.abs_diff(pts.nseconds()) > 1_000_000)
        {
            self.quiet = 0;
        }
        self.end = Some(pts.nseconds().saturating_add(duration));
        let drop = quiet && duration > 0 && self.quiet >= KEEP_NS;
        self.quiet = if quiet {
            self.quiet.saturating_add(duration)
        } else {
            0
        };
        let output_pts = gst::ClockTime::from_nseconds(pts.nseconds().saturating_sub(self.skipped));
        if drop
            && let (Some(compact), Some(source_end)) = (
                segment.to_stream_time(output_pts),
                pts.checked_add(gst::ClockTime::from_nseconds(duration))
                    .and_then(|end| segment.to_stream_time(end)),
            )
        {
            self.skipped = self.skipped.saturating_add(duration);
            let jump = (
                compact.nseconds(),
                source_end.nseconds().saturating_sub(compact.nseconds()),
            );
            if self.jumps.back().is_some_and(|last| last.0 == jump.0) {
                *self.jumps.back_mut().unwrap() = jump;
            } else {
                self.jumps.push_back(jump);
            }
            return gst::PadProbeReturn::Drop;
        }
        if self.skipped > 0 {
            let buffer = buffer.make_mut();
            buffer.set_pts(output_pts);
            buffer.set_dts(buffer.dts().map(|dts| {
                gst::ClockTime::from_nseconds(dts.nseconds().saturating_sub(self.skipped))
            }));
        }
        gst::PadProbeReturn::Ok
    }
}

#[derive(Clone)]
pub(crate) struct Silence(Arc<Mutex<Timeline>>);
impl Silence {
    pub fn filter(enabled: bool) -> Result<(gst::Bin, Self)> {
        let state = Self(Arc::new(Mutex::new(Timeline {
            enabled,
            ..Timeline::default()
        })));
        let bin = gst::Bin::new();
        let convert = gst::ElementFactory::make("audioconvert").build()?;
        let caps = gst::ElementFactory::make("capsfilter")
            .property(
                "caps",
                gst::Caps::builder("audio/x-raw")
                    .field("format", "F32LE")
                    .field("layout", "interleaved")
                    .build(),
            )
            .build()?;
        let tempo = gst::ElementFactory::make("scaletempo").build()?;
        bin.add_many([&convert, &caps, &tempo])?;
        gst::Element::link_many([&convert, &caps, &tempo])?;
        let data = state.clone();
        caps.static_pad("src").unwrap().add_probe(
            gst::PadProbeType::BUFFER | gst::PadProbeType::EVENT_DOWNSTREAM,
            move |_, info| {
                let mut timeline = data.0.lock().unwrap();
                if let Some(event) = info.event()
                    && let gst::EventView::Segment(event) = event.view()
                {
                    timeline.reset(event.segment().downcast_ref::<gst::ClockTime>().cloned());
                }
                if let Some(buffer) = info.buffer_mut() {
                    return timeline.process(buffer);
                }
                gst::PadProbeReturn::Ok
            },
        );
        bin.add_pad(&gst::GhostPad::with_target(
            &convert.static_pad("sink").unwrap(),
        )?)?;
        bin.add_pad(&gst::GhostPad::with_target(
            &tempo.static_pad("src").unwrap(),
        )?)?;
        Ok((bin, state))
    }
    pub fn set_enabled(&self, enabled: bool) {
        self.0.lock().unwrap().enabled = enabled;
    }
    pub fn source_position(&self, position: gst::ClockTime) -> gst::ClockTime {
        gst::ClockTime::from_nseconds(
            self.0
                .lock()
                .unwrap()
                .source_position(position.nseconds())
                .min(gst::ClockTime::MAX.nseconds()),
        )
    }
    pub fn take_discontinuity(&self) -> bool {
        std::mem::take(&mut self.0.lock().unwrap().discontinuity)
    }
}

#[cfg(test)]
mod tests;
