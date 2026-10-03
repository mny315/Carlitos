use anyhow::{Context, Result, ensure};
use windows::{
    Win32::Media::MediaFoundation::*,
    core::{GUID, HSTRING},
};

const AUDIO: u32 = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;

pub(crate) struct Decoder {
    reader: IMFSourceReader,
    pub rate: u32,
    pub channels: u16,
    pub duration: Option<u64>,
    pub end: bool,
    trim_before: u64,
}

impl Decoder {
    pub fn open(uri: &str) -> Result<Self> {
        super::initialize_media()?;
        let path = crate::library::local_path(uri).context("Expected a local audio file")?;
        unsafe {
            let reader = MFCreateSourceReaderFromURL(&HSTRING::from(path.as_os_str()), None)?;
            reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false)?;
            reader.SetStreamSelection(AUDIO, true)?;
            let media = MFCreateMediaType()?;
            media.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Audio)?;
            media.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)?;
            media.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)?;
            reader.SetCurrentMediaType(AUDIO, None, &media)?;
            let actual = reader.GetCurrentMediaType(AUDIO)?;
            let rate = actual.GetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND)?;
            let channels = actual.GetUINT32(&MF_MT_AUDIO_NUM_CHANNELS)?;
            super::tempo::validate_format(rate, channels)?;
            ensure!(
                actual.GetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE)? == 16,
                "Expected 16-bit PCM"
            );
            let duration = reader
                .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
                .ok()
                .and_then(|v| u64::try_from(&v).ok())
                .map(|v| v / 10_000);
            Ok(Self {
                reader,
                rate,
                channels: channels as u16,
                duration,
                end: false,
                trim_before: 0,
            })
        }
    }

    pub fn seek(&mut self, millis: u64) -> Result<()> {
        let time = millis.min(i64::MAX as u64 / 10_000) as i64 * 10_000;
        unsafe {
            self.reader
                .SetCurrentPosition(&GUID::zeroed(), &time.into())?;
        }
        self.trim_before = millis.saturating_mul(u64::from(self.rate)) / 1000;
        self.end = false;
        Ok(())
    }

    /// One source-reader packet. Seeking can land before the requested sample.
    pub fn read(&mut self) -> Result<Vec<i16>> {
        if self.end {
            return Ok(Vec::new());
        }
        unsafe {
            let mut flags = 0;
            let mut timestamp = 0;
            let mut sample = None;
            self.reader.ReadSample(
                AUDIO,
                0,
                None,
                Some(&mut flags),
                Some(&mut timestamp),
                Some(&mut sample),
            )?;
            ensure!(
                flags & MF_SOURCE_READERF_ERROR.0 as u32 == 0,
                "Audio decoder failed"
            );
            ensure!(
                flags & MF_SOURCE_READERF_CURRENTMEDIATYPECHANGED.0 as u32 == 0,
                "Audio format changed within the file"
            );
            self.end = flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0;
            let Some(sample) = sample else {
                return Ok(Vec::new());
            };
            let buffer = sample.ConvertToContiguousBuffer()?;
            let mut ptr = std::ptr::null_mut();
            let mut len = 0;
            buffer.Lock(&mut ptr, None, Some(&mut len))?;
            // Copy while locked; do not borrow unaligned decoder memory as i16.
            let bytes = if len == 0 {
                &[][..]
            } else {
                std::slice::from_raw_parts(ptr, len as usize)
            };
            let pcm = crate::audio_processing::decode_pcm16(bytes, self.channels);
            // Release the Media Foundation buffer even when validation fails.
            buffer.Unlock()?;
            let mut pcm = pcm?;
            let frame = (timestamp.max(0) as u64).saturating_mul(u64::from(self.rate)) / 10_000_000;
            let trim =
                self.trim_before
                    .saturating_sub(frame)
                    .min((pcm.len() / usize::from(self.channels)) as u64) as usize;
            pcm.drain(..trim * usize::from(self.channels));
            Ok(pcm)
        }
    }
}
