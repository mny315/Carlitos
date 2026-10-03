//! Small ownership wrapper for the statically compiled Apache-2.0 Sonic library.
use anyhow::{Result, ensure};
use std::ffi::c_void;
unsafe extern "C" {
    fn sonicCreateStream(rate: i32, channels: i32) -> *mut c_void;
    fn sonicDestroyStream(stream: *mut c_void);
    fn sonicSetSpeed(stream: *mut c_void, speed: f32);
    fn sonicWriteShortToStream(stream: *mut c_void, samples: *const i16, frames: i32) -> i32;
    fn sonicReadShortFromStream(stream: *mut c_void, samples: *mut i16, frames: i32) -> i32;
    fn sonicFlushStream(stream: *mut c_void) -> i32;
    fn sonicSamplesAvailable(stream: *mut c_void) -> i32;
}
pub(crate) struct Tempo {
    stream: *mut c_void,
    channels: usize,
}

pub(super) fn validate_format(rate: u32, channels: u32) -> Result<()> {
    // Match the supported decoder/output range. Sonic assumes positive pitch
    // periods and channel counts; unchecked values can divide by zero in C.
    ensure!(
        (1000..=192_000).contains(&rate) && (1..=8).contains(&channels),
        "Unsupported audio format"
    );
    Ok(())
}

impl Tempo {
    pub fn new(rate: u32, channels: u16, speed: f64) -> Result<Self> {
        validate_format(rate, u32::from(channels))?;
        ensure!(
            speed.is_finite()
                && (crate::player::MIN_RATE..=crate::player::MAX_RATE).contains(&speed),
            "Unsupported playback speed"
        );
        let stream = unsafe { sonicCreateStream(rate as i32, i32::from(channels)) };
        ensure!(
            !stream.is_null(),
            "Could not allocate audio tempo processor"
        );
        unsafe {
            sonicSetSpeed(stream, speed as f32);
        }
        Ok(Self {
            stream,
            channels: usize::from(channels),
        })
    }
    pub fn write(&mut self, pcm: &[i16]) -> Result<()> {
        ensure!(
            unsafe {
                sonicWriteShortToStream(
                    self.stream,
                    pcm.as_ptr(),
                    (pcm.len() / self.channels) as i32,
                )
            } != 0,
            "Audio tempo processor ran out of memory"
        );
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        ensure!(
            unsafe { sonicFlushStream(self.stream) } != 0,
            "Could not finish audio processing"
        );
        Ok(())
    }
    pub fn read(&mut self, max_frames: usize) -> Vec<i16> {
        let frames = (unsafe { sonicSamplesAvailable(self.stream) }).max(0) as usize;
        let frames = frames.min(max_frames);
        if frames == 0 {
            return Vec::new();
        }
        let mut pcm = vec![0; frames * self.channels];
        let read = unsafe { sonicReadShortFromStream(self.stream, pcm.as_mut_ptr(), frames as i32) }
            .max(0) as usize;
        pcm.truncate(read * self.channels);
        pcm
    }
}
impl Drop for Tempo {
    fn drop(&mut self) {
        unsafe {
            sonicDestroyStream(self.stream);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Context;

    #[test]
    fn invalid_stream_parameters_are_rejected_before_calling_sonic() {
        for rate in [0, 1, 64, 999, 192_001, u32::MAX] {
            assert!(Tempo::new(rate, 1, 1.5).is_err(), "rate {rate}");
        }
        for channels in [0, 9, u16::MAX] {
            assert!(Tempo::new(16_000, channels, 1.5).is_err());
        }
        for speed in [f64::NAN, f64::INFINITY, 0., 0.49, 3.01] {
            assert!(Tempo::new(16_000, 1, speed).is_err());
        }
    }

    #[test]
    fn supported_formats_preserve_channels_and_expected_duration() -> Result<()> {
        for rate in [1000, 8000, 44_100, 192_000] {
            for channels in [1, 2, 8] {
                for speed in [0.5, 1., 1.75, 3.] {
                    let mut tempo = Tempo::new(rate, channels, speed)?;
                    let pcm: Vec<_> = (0..rate)
                        .flat_map(|frame| {
                            let value = ((f64::from(frame) * 120. * std::f64::consts::TAU
                                / f64::from(rate))
                            .sin()
                                * 8000.) as i16;
                            std::iter::repeat_n(value, usize::from(channels))
                        })
                        .collect();
                    for chunk in pcm.chunks(257 * usize::from(channels)) {
                        tempo.write(chunk).with_context(|| {
                            format!("write: {rate} Hz, {channels} channels, {speed}x")
                        })?;
                    }
                    tempo.finish().with_context(|| {
                        format!("flush: {rate} Hz, {channels} channels, {speed}x")
                    })?;
                    let mut frames = 0;
                    loop {
                        let output = tempo.read(113);
                        if output.is_empty() {
                            break;
                        }
                        assert!(output.len().is_multiple_of(usize::from(channels)));
                        for frame in output.chunks_exact(usize::from(channels)) {
                            assert!(frame.iter().all(|sample| *sample == frame[0]));
                        }
                        frames += output.len() / usize::from(channels);
                    }
                    let expected = (f64::from(rate) / speed).round() as usize;
                    // Pitch periods are whole frames; the lowest supported
                    // sample rate has noticeably coarser tempo quantization.
                    let tolerance = rate as usize / if rate < 8000 { 10 } else { 25 };
                    assert!(
                        frames.abs_diff(expected) <= tolerance,
                        "{rate} Hz, {channels} channels, {speed}x: {frames}, expected {expected}"
                    );
                }
            }
        }
        Ok(())
    }
}
