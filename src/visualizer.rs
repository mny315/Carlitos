//! Four broad, bounded voice bands. No FFT, audio copies or UI messages.
#[cfg(target_os = "linux")]
mod linux;

#[cfg(any(not(any(target_os = "android", target_os = "linux")), test))]
use std::sync::{
    OnceLock,
    atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
};
#[cfg(any(not(any(target_os = "android", target_os = "linux")), test))]
use std::time::Instant;

#[cfg(any(not(any(target_os = "android", target_os = "linux")), test))]
struct Meter {
    enabled: AtomicBool,
    levels: [AtomicU32; 4],
    updated: AtomicU64,
    clock: OnceLock<Instant>,
}
#[cfg(any(not(any(target_os = "android", target_os = "linux")), test))]
impl Meter {
    const fn new() -> Self {
        Self {
            enabled: AtomicBool::new(false),
            levels: [const { AtomicU32::new(0) }; 4],
            updated: AtomicU64::new(0),
            clock: OnceLock::new(),
        }
    }
    fn now(&self) -> u64 {
        self.clock.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
    }
    fn enable(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        for level in &self.levels {
            level.store(0, Ordering::Relaxed);
        }
        self.updated.store(0, Ordering::Relaxed);
    }
    fn begin(&self) -> bool {
        // Background playback only pays for this atomic load.
        if !self.enabled.load(Ordering::Relaxed) {
            return false;
        }
        let now = self.now();
        let last = self.updated.load(Ordering::Relaxed);
        (last == 0 || now.saturating_sub(last) >= 80)
            && self
                .updated
                .compare_exchange(last, now, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
    }
    fn publish(&self, levels: [f32; 4]) {
        for (output, level) in self.levels.iter().zip(levels) {
            output.store(level.to_bits(), Ordering::Relaxed);
        }
    }
    fn read(&self) -> [f32; 4] {
        if !self.enabled.load(Ordering::Relaxed)
            || self.updated.load(Ordering::Relaxed) == 0
            || self
                .now()
                .saturating_sub(self.updated.load(Ordering::Relaxed))
                > 200
        {
            return [0.; 4];
        }
        self.levels
            .each_ref()
            .map(|level| f32::from_bits(level.load(Ordering::Relaxed)))
    }
}
#[cfg(not(any(target_os = "android", target_os = "linux")))]
static METER: Meter = Meter::new();

pub(crate) fn set_enabled(enabled: bool) {
    #[cfg(target_os = "linux")]
    linux::set_enabled(enabled);
    #[cfg(not(any(target_os = "android", target_os = "linux")))]
    METER.enable(enabled);
    #[cfg(target_os = "android")]
    let _ = crate::android::bridge::visualizer_enabled(enabled);
}
pub(crate) fn bands() -> crate::VoiceBands {
    #[cfg(target_os = "android")]
    let levels = {
        let packed = crate::android::bridge::voice_bands().unwrap_or(0);
        std::array::from_fn::<_, 4, _>(|i| ((packed >> (i * 8)) & 255) as f32 / 255.)
    };
    #[cfg(target_os = "linux")]
    let levels = linux::read();
    #[cfg(not(any(target_os = "android", target_os = "linux")))]
    let levels = METER.read();
    crate::VoiceBands {
        bass: levels[0],
        body: levels[1],
        clarity: levels[2],
        air: levels[3],
    }
}

/// Box-decimate a bounded PCM window, then split it with four one-pole filters.
/// Stereo channels retain independent filter state, including opposite phases.
#[cfg(any(not(target_os = "android"), test))]
pub(crate) fn analyze(
    sample_rate: u32,
    frames: usize,
    channels: usize,
    mut sample: impl FnMut(usize, usize) -> f32,
) -> [f32; 4] {
    if sample_rate == 0 || channels == 0 {
        return [0.; 4];
    }
    let channels = channels.min(2);
    let stride = (sample_rate / 12_000).max(1) as usize;
    let count = (frames / stride).min(512 / channels);
    if count == 0 {
        return [0.; 4];
    }
    let rate = sample_rate as f32 / stride as f32;
    let alpha =
        [60., 240., 700., 1800.].map(|cutoff| 1. - (-std::f32::consts::TAU * cutoff / rate).exp());
    let mut states = [[0f32; 4]; 2];
    let mut powers = [0f32; 4];
    let mut input_power = 0.;
    for frame in 0..count {
        for (channel, state) in states.iter_mut().enumerate().take(channels) {
            let mut value = 0.;
            for offset in 0..stride {
                let x = sample(frame * stride + offset, channel);
                value += if x.is_finite() { x.clamp(-1., 1.) } else { 0. };
            }
            value /= stride as f32;
            state[0] += alpha[0] * (value - state[0]);
            value -= state[0];
            input_power += value * value;
            for i in 1..4 {
                state[i] += alpha[i] * (value - state[i]);
            }
            let bands = [
                state[1],
                state[2] - state[1],
                state[3] - state[2],
                value - state[3],
            ];
            for (power, band) in powers.iter_mut().zip(bands) {
                *power += band * band;
            }
        }
    }
    if input_power < f32::EPSILON {
        return [0.; 4];
    }
    let rms = (input_power / (count * channels) as f32).sqrt();
    let strength = ((rms.sqrt() - 0.035) * 2.).clamp(0., 1.);
    let gains = [1.35, 1.9, 2.3, 1.45];
    std::array::from_fn(|i| strength * ((powers[i] / input_power).sqrt() * gains[i]).clamp(0., 1.))
}

#[cfg(target_os = "linux")]
pub(crate) fn attach(pipeline: &gst::Element, output: &gst::Pad) {
    linux::attach(pipeline, output);
}
#[cfg(windows)]
pub(crate) fn observe_pcm16(samples: &[i16], channels: usize, sample_rate: u32) {
    if channels > 0 && METER.begin() {
        METER.publish(analyze(
            sample_rate,
            samples.len() / channels,
            channels,
            |frame, channel| f32::from(samples[frame * channels + channel]) / 32768.,
        ));
    }
}

/// Blur this tiny alpha mask once; every frame only stretches nine textures.
pub(crate) fn glow() -> slint::Image {
    let mut mask = image::RgbaImage::from_pixel(64, 128, image::Rgba([255, 255, 255, 0]));
    for (x, y, pixel) in mask.enumerate_pixels_mut() {
        let dx = x as f32 - 31.5;
        let dy = (y as f32 - 63.5).abs().max(30.) - 30.;
        if dx * dx + dy * dy < 14. * 14. {
            pixel.0[3] = 255;
        }
    }
    let blurred = image::imageops::blur(&mask, 7.);
    slint::Image::from_rgba8(
        slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(blurred.as_raw(), 64, 128),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_meter_is_bounded_throttled_and_completely_disabled() {
        let meter = Meter::new();
        assert!(!meter.begin());
        meter.enable(true);
        assert!(meter.begin());
        meter.publish([0.4, 0.3, 0.2, 0.1]);
        assert_eq!(meter.read(), [0.4, 0.3, 0.2, 0.1]);
        assert!(!meter.begin());
        meter.enable(false);
        assert!(!meter.begin());
        assert_eq!(meter.read(), [0.; 4]);
        meter.enable(true);
        assert_eq!(meter.read(), [0.; 4]);
        assert!(meter.begin());
        meter.publish([0.; 4]);
        assert_eq!(meter.read(), [0.; 4]);
    }
    #[test]
    fn voice_bands_distinguish_frequencies_without_consuming_unbounded_audio() {
        for (band, frequency) in [90., 450., 1200., 3600.].into_iter().enumerate() {
            let mut reads = 0;
            let levels = analyze(48_000, 100_000, 2, |frame, channel| {
                reads += 1;
                (frame as f32 * std::f32::consts::TAU * frequency / 48_000.).sin()
                    * if channel == 0 { 0.12 } else { -0.12 }
            });
            assert!(reads <= 2048);
            assert!(
                levels
                    .iter()
                    .all(|value| value.is_finite() && (0.0..=1.0).contains(value))
            );
            for (other, value) in levels.iter().enumerate() {
                if other != band {
                    assert!(levels[band] > *value, "{frequency} Hz: {levels:?}");
                }
            }
        }
        assert_eq!(analyze(12_000, 512, 1, |_, _| 0.), [0.; 4]);
        assert_eq!(analyze(12_000, 512, 1, |_, _| f32::NAN), [0.; 4]);
    }
}
