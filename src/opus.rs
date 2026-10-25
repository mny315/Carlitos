//! Streaming Opus decoding for Windows, independent of installed media extensions.
use anyhow::{Context, Result, bail, ensure};
use std::{collections::VecDeque, fs::File, path::Path, ptr::NonNull};
use symphonia::core::{
    codecs::CODEC_TYPE_OPUS,
    errors::Error,
    formats::{FormatOptions, FormatReader, Packet, SeekMode, SeekTo},
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};

const RATE: u32 = 48_000;
const MAX_FRAMES: usize = 5760; // The maximum Opus packet duration is 120 ms.

struct Codec(NonNull<opusic_sys::OpusDecoder>);

impl Drop for Codec {
    fn drop(&mut self) {
        // This pointer belongs exclusively to this decoder.
        unsafe { opusic_sys::opus_decoder_destroy(self.0.as_ptr()) };
    }
}

pub(crate) struct Decoder {
    format: Box<dyn FormatReader>,
    codec: Codec,
    track: u32,
    pending: VecDeque<Packet>,
    demux_delay: u64,
    first_ts: u64,
    start: u64,
    stop: u64,
    trim_before: u64,
    next_frame: Option<u64>,
    pub channels: u16,
    pub duration: u64,
    pub end: bool,
}

impl Decoder {
    pub fn open(path: &Path) -> Result<Self> {
        let stream = MediaSourceStream::new(Box::new(File::open(path)?), Default::default());
        let mut hint = Hint::new();
        hint.with_extension("opus");
        let mut format = symphonia::default::get_probe()
            .format(
                &hint,
                stream,
                &FormatOptions {
                    enable_gapless: true,
                    ..Default::default()
                },
                &MetadataOptions::default(),
            )?
            .format;
        let track = format
            .default_track()
            .context("Missing Opus audio stream")?;
        let params = &track.codec_params;
        ensure!(params.codec == CODEC_TYPE_OPUS, "Expected an Opus stream");
        let header = params
            .extra_data
            .as_deref()
            .context("Missing Opus header")?;
        ensure!(
            header.len() >= 19 && &header[..8] == b"OpusHead" && header[8] <= 15,
            "Invalid Opus header"
        );
        let channels = u16::from(header[9]);
        ensure!(
            (1..=2).contains(&channels) && header[18] == 0,
            "Only mono and stereo Opus streams are supported"
        );
        let pre_skip = u64::from(u16::from_le_bytes([header[10], header[11]]));
        let gain = i16::from_le_bytes([header[16], header[17]]);
        let first_ts = params.start_ts;
        let start = first_ts.max(pre_skip);
        let stop = first_ts
            .checked_add(params.n_frames.context("Missing Opus duration")?)
            .context("Invalid Opus duration")?;
        ensure!(stop > start, "Empty Opus stream");
        let track = track.id;

        // Ogg's demuxer normalizes timestamps and trims partial first/last pages.
        // Recover its initial offset so OpusHead pre-skip is applied exactly once,
        // including short files whose first audio page is also their final page.
        let mut pending = VecDeque::new();
        let mut timestamp = first_ts;
        let mut buffered_bytes = 0usize;
        let demux_delay = loop {
            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(error))
                    if error.kind() == std::io::ErrorKind::UnexpectedEof && !pending.is_empty() =>
                {
                    break 0;
                }
                Err(error) => return Err(error.into()),
            };
            if packet.track_id() != track {
                continue;
            }
            buffered_bytes += packet.data.len();
            ensure!(
                buffered_bytes <= 1_048_576,
                "Excessive Opus initial padding"
            );
            let frames = packet.dur() + u64::from(packet.trim_start + packet.trim_end);
            let offset = timestamp
                .saturating_add(u64::from(packet.trim_start))
                .saturating_sub(packet.ts());
            // The initial packet timestamp may have saturated at zero before
            // the demuxer inspected the final page. Use a later timestamp.
            let reliable = packet.ts() > first_ts
                || (packet.trim_start > 0 && u64::from(packet.trim_start) < frames);
            timestamp = timestamp.saturating_add(frames);
            pending.push_back(packet);
            if reliable {
                break offset;
            }
        };
        let mut error = 0;
        // libopus owns the allocation. The channel count was validated above.
        let pointer = unsafe {
            opusic_sys::opus_decoder_create(RATE as i32, i32::from(channels), &mut error)
        };
        let codec = Codec(NonNull::new(pointer).context("Cannot allocate Opus decoder")?);
        ensure!(error == opusic_sys::OPUS_OK, "Opus initialization: {error}");
        let result = unsafe {
            opusic_sys::opus_decoder_ctl(
                codec.0.as_ptr(),
                opusic_sys::OPUS_SET_GAIN_REQUEST,
                i32::from(gain),
            )
        };
        ensure!(result == opusic_sys::OPUS_OK, "Opus output gain: {result}");
        Ok(Self {
            format,
            codec,
            track,
            pending,
            demux_delay,
            first_ts,
            start,
            stop,
            trim_before: start,
            next_frame: Some(first_ts),
            channels,
            duration: (stop - start).saturating_mul(1000) / u64::from(RATE),
            end: false,
        })
    }

    pub fn seek(&mut self, millis: u64) -> Result<()> {
        let target = self
            .start
            .saturating_add(millis.saturating_mul(u64::from(RATE)) / 1000)
            .min(self.stop);
        // Opus requires at least 80 ms of decoding before the requested sample.
        let preroll = target.saturating_sub(3840).max(self.start);
        let seek = self.format.seek(
            SeekMode::Accurate,
            SeekTo::TimeStamp {
                ts: preroll.saturating_sub(self.demux_delay).max(self.first_ts),
                track_id: self.track,
            },
        )?;
        let result = unsafe {
            opusic_sys::opus_decoder_ctl(self.codec.0.as_ptr(), opusic_sys::OPUS_RESET_STATE)
        };
        ensure!(result == opusic_sys::OPUS_OK, "Opus seek reset: {result}");
        self.pending.clear();
        self.next_frame = (seek.actual_ts == self.first_ts).then_some(self.first_ts);
        self.trim_before = target;
        self.end = target == self.stop;
        Ok(())
    }

    /// Decode at most one packet; memory use is independent of book length.
    pub fn read(&mut self) -> Result<Vec<i16>> {
        if self.end {
            return Ok(Vec::new());
        }
        let packet = loop {
            let packet = if let Some(packet) = self.pending.pop_front() {
                packet
            } else {
                match self.format.next_packet() {
                    Ok(packet) => packet,
                    Err(Error::IoError(error))
                        if error.kind() == std::io::ErrorKind::UnexpectedEof =>
                    {
                        self.end = true;
                        return Ok(Vec::new());
                    }
                    Err(error) => return Err(error.into()),
                }
            };
            if packet.track_id() == self.track {
                break packet;
            }
        };
        ensure!(!packet.data.is_empty(), "Empty Opus packet");
        let bytes = i32::try_from(packet.data.len()).context("Oversized Opus packet")?;
        let channels = usize::from(self.channels);
        let mut pcm = vec![0i16; MAX_FRAMES * channels];
        // Both slices remain valid for the call, with space for 120 ms per channel.
        let frames = unsafe {
            opusic_sys::opus_decode(
                self.codec.0.as_ptr(),
                packet.data.as_ptr(),
                bytes,
                pcm.as_mut_ptr(),
                MAX_FRAMES as i32,
                0,
            )
        };
        if frames < 0 {
            bail!("Opus decoding failed: {frames}");
        }
        let frame = self.next_frame.unwrap_or_else(|| {
            packet
                .ts()
                .saturating_add(self.demux_delay)
                .saturating_sub(u64::from(packet.trim_start))
        });
        self.next_frame = Some(frame.saturating_add(frames as u64));
        let from = self.trim_before.saturating_sub(frame).min(frames as u64) as usize;
        let to = self.stop.saturating_sub(frame).min(frames as u64) as usize;
        pcm.truncate(to.max(from) * channels);
        pcm.drain(..from * channels);
        if frame.saturating_add(frames as u64) >= self.stop {
            self.end = true;
        }
        Ok(pcm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(decoder: &mut Decoder) -> Result<Vec<i16>> {
        let mut pcm = Vec::new();
        for _ in 0..1000 {
            pcm.extend(decoder.read()?);
            if decoder.end {
                return Ok(pcm);
            }
        }
        bail!("Opus decoder failed to finish")
    }

    #[test]
    fn opus_decodes_seeks_and_trims_encoder_padding() -> Result<()> {
        let mut decoder = Decoder::open(Path::new("tests/fixtures/import.opus"))?;
        assert_eq!(decoder.channels, 2);
        assert_eq!(decoder.duration, 1000);
        let pcm = decode(&mut decoder)?;
        assert_eq!(pcm.len(), 48_000 * 2);
        assert!(pcm.iter().any(|s| s.unsigned_abs() > 100));
        decoder.seek(400)?;
        assert_eq!(decode(&mut decoder)?.len(), 28_800 * 2);
        decoder.seek(0)?;
        assert_eq!(decode(&mut decoder)?, pcm);
        decoder.seek(9999)?;
        assert!(decoder.end);
        assert!(decoder.read()?.is_empty());
        Ok(())
    }

    #[test]
    fn rejects_invalid_opus() -> Result<()> {
        let file = tempfile::NamedTempFile::new()?;
        std::fs::write(file.path(), b"OpusHead\x01\x01 invalid stream")?;
        assert!(Decoder::open(file.path()).is_err());
        Ok(())
    }

    #[test]
    fn stereo_and_single_page_opus_keep_sample_accurate_duration() -> Result<()> {
        for (file, channels, millis) in [
            ("import-stereo.opus", 2, 1200),
            ("import-short.opus", 1, 20),
        ] {
            let mut decoder = Decoder::open(&Path::new("tests/fixtures").join(file))?;
            assert_eq!(decoder.channels, channels, "{file}");
            assert_eq!(decoder.duration, millis, "{file}");
            let pcm = decode(&mut decoder)?;
            assert_eq!(
                pcm.len() as u64,
                millis * 48 * u64::from(channels),
                "{file}"
            );
            let seek = millis / 2;
            decoder.seek(seek)?;
            assert_eq!(
                decode(&mut decoder)?.len() as u64,
                (millis - seek) * 48 * u64::from(channels),
                "{file} after seek"
            );
            decoder.seek(0)?;
            assert_eq!(decode(&mut decoder)?, pcm, "{file} after rewind");
        }
        Ok(())
    }
}
