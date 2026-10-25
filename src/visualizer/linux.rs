//! Read post-tempo PCM on the streaming thread, display it on the output clock.
use gst::prelude::*;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

const INTERVAL: u64 = 80_000_000;
const EXPIRES: u64 = 200_000_000;
const CAPACITY: usize = 64;

#[derive(Default)]
struct Samples {
    pending: VecDeque<(u64, [f32; 4])>,
    current: Option<(u64, [f32; 4])>,
    sampled: Option<u64>,
}
impl Samples {
    fn ready(&self, time: u64) -> bool {
        self.pending.len() < CAPACITY
            && self
                .sampled
                .is_none_or(|last| time.saturating_sub(last) >= INTERVAL)
    }
    fn push(&mut self, time: u64, levels: [f32; 4]) {
        self.sampled = Some(time);
        self.pending.push_back((time, levels));
    }
    fn read(&mut self, time: u64) -> [f32; 4] {
        while self.pending.front().is_some_and(|frame| frame.0 <= time) {
            self.current = self.pending.pop_front();
        }
        self.current
            .filter(|frame| time >= frame.0 && time - frame.0 <= EXPIRES)
            .map_or([0.; 4], |frame| frame.1)
    }
}

#[derive(Default)]
struct Timeline {
    segment: Option<gst::FormattedSegment<gst::ClockTime>>,
    rate: u32,
    channels: usize,
    latency: u64,
    samples: Samples,
}

struct Output {
    enabled: AtomicBool,
    pipeline: gst::glib::WeakRef<gst::Element>,
    timeline: Mutex<Timeline>,
}
impl Output {
    fn attach(pipeline: &gst::Element, pad: &gst::Pad, enabled: bool) -> Arc<Self> {
        let output = Arc::new(Self {
            enabled: AtomicBool::new(enabled),
            pipeline: pipeline.downgrade(),
            timeline: Mutex::new(Timeline::default()),
        });
        let observer = output.clone();
        pad.add_probe(
            gst::PadProbeType::BUFFER
                | gst::PadProbeType::EVENT_DOWNSTREAM
                | gst::PadProbeType::EVENT_UPSTREAM
                | gst::PadProbeType::EVENT_FLUSH,
            move |_, info| {
                if let Some(event) = info.event() {
                    let mut state = observer.timeline.lock().unwrap();
                    match event.view() {
                        gst::EventView::Segment(event) => {
                            state.segment =
                                event.segment().downcast_ref::<gst::ClockTime>().cloned();
                            state.samples = Samples::default();
                        }
                        gst::EventView::FlushStart(_) => {
                            state.segment = None;
                            state.samples = Samples::default();
                        }
                        gst::EventView::Caps(event) => {
                            state.rate = 0;
                            state.channels = 0;
                            if let Some(format) = event.caps().structure(0)
                                && format.get::<&str>("format") == Ok("F32LE")
                            {
                                state.rate = format.get::<i32>("rate").unwrap_or(0).max(0) as u32;
                                state.channels =
                                    format.get::<i32>("channels").unwrap_or(0).max(0) as usize;
                            }
                        }
                        gst::EventView::Latency(event) => {
                            state.latency = event.latency().nseconds();
                        }
                        _ => {}
                    }
                }
                if let Some(buffer) = info.buffer() {
                    observer.observe(buffer);
                }
                gst::PadProbeReturn::Ok
            },
        );
        output
    }

    fn enable(&self, enabled: bool) {
        // Already queued audio remains valid across pause/resume; its clock
        // stops with the sink. Seeks and new segments discard it separately.
        self.enabled.store(enabled, Ordering::Relaxed);
    }

    fn observe(&self, buffer: &gst::BufferRef) {
        // No PCM reads, clocks or locking during background playback.
        if !self.enabled.load(Ordering::Relaxed) {
            return;
        }
        let mut state = self.timeline.lock().unwrap();
        if !self.enabled.load(Ordering::Relaxed) || state.rate == 0 || state.channels == 0 {
            return;
        }
        let Some(pts) = buffer.pts() else { return };
        let Some(segment) = state.segment.as_ref() else {
            return;
        };
        if segment.to_running_time(pts).is_none() {
            return;
        }
        let frames = buffer.size() / 4 / state.channels;
        let stride = (state.rate / 12_000).max(1) as usize;
        let count = (frames / stride).min(512 / state.channels.min(2));
        if count == 0 {
            return;
        }
        // Wait until the analyzed window has actually played, including
        // the pipeline latency. Decoder arrival time is never a frame clock.
        let duration = gst::ClockTime::from_nseconds(
            (count * stride) as u64 * 1_000_000_000 / u64::from(state.rate),
        );
        let Some(time) = pts
            .checked_add(duration)
            .and_then(|end| segment.to_running_time(end))
        else {
            return;
        };
        if !state.samples.ready(time.nseconds()) {
            return;
        }
        let Ok(map) = buffer.map_readable() else {
            return;
        };
        let samples = map.as_chunks::<4>().0;
        let levels = super::analyze(state.rate, frames, state.channels, |frame, channel| {
            f32::from_le_bytes(samples[frame * state.channels + channel])
        });
        state.samples.push(time.nseconds(), levels);
    }

    fn read(&self) -> [f32; 4] {
        if !self.enabled.load(Ordering::Relaxed) {
            return [0.; 4];
        }
        let Some(pipeline) = self.pipeline.upgrade() else {
            return [0.; 4];
        };
        if pipeline.current_state() != gst::State::Playing {
            return [0.; 4];
        }
        let Some(time) = pipeline.current_running_time() else {
            return [0.; 4];
        };
        let mut state = self.timeline.lock().unwrap();
        time.nseconds()
            .checked_sub(state.latency)
            .map_or([0.; 4], |time| state.samples.read(time))
    }
}

// The registry keeps only a weak pipeline reference, so replacing a book
// cannot retain its decoder or let an old pipeline publish into the new one.
static OUTPUT: Mutex<(bool, Option<Arc<Output>>)> = Mutex::new((false, None));

pub(super) fn attach(pipeline: &gst::Element, pad: &gst::Pad) {
    let mut registry = OUTPUT.lock().unwrap();
    registry.1 = Some(Output::attach(pipeline, pad, registry.0));
}
pub(super) fn set_enabled(enabled: bool) {
    let mut registry = OUTPUT.lock().unwrap();
    registry.0 = enabled;
    if let Some(output) = &registry.1 {
        output.enable(enabled);
    }
}
pub(super) fn read() -> [f32; 4] {
    let output = OUTPUT.lock().unwrap().1.clone();
    output.map_or([0.; 4], |output| output.read())
}

#[cfg(test)]
mod tests;
