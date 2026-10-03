use anyhow::{Context, Result};
use std::{collections::VecDeque, time::Instant};
use windows::{
    Win32::Media::Audio::{XAudio2::*, *},
    core::{HRESULT, Interface, PCWSTR},
};

#[link(name = "xaudio2")]
unsafe extern "system" {
    fn XAudio2Create(engine: *mut *mut std::ffi::c_void, flags: u32, processor: u32) -> HRESULT;
}

pub(crate) struct Output {
    voice: Option<IXAudio2SourceVoice>,
    master: Option<IXAudio2MasteringVoice>,
    _engine: Option<IXAudio2>,
    // XAudio2 borrows these allocations until the corresponding buffers finish.
    buffers: VecDeque<Vec<i16>>,
    rate: u32,
    channels: usize,
    fake_position: u64,
    fake_fraction: f64,
    submitted_frames: u64,
    updated: Instant,
    playing: bool,
}
impl Output {
    pub fn new(rate: u32, channels: u16, fake: bool) -> Result<Self> {
        let mut result = Self {
            voice: None,
            master: None,
            _engine: None,
            buffers: VecDeque::new(),
            rate,
            channels: usize::from(channels),
            fake_position: 0,
            fake_fraction: 0.,
            submitted_frames: 0,
            updated: Instant::now(),
            playing: false,
        };
        if !fake {
            unsafe {
                let mut raw = std::ptr::null_mut();
                XAudio2Create(&mut raw, 0, XAUDIO2_DEFAULT_PROCESSOR).ok()?;
                let engine = IXAudio2::from_raw(raw);
                engine.CreateMasteringVoice(
                    &mut result.master,
                    0,
                    0,
                    0,
                    PCWSTR::null(),
                    None,
                    AudioCategory_Media,
                )?;
                result._engine = Some(engine);
                let format = WAVEFORMATEX {
                    wFormatTag: 1,
                    nChannels: channels,
                    nSamplesPerSec: rate,
                    nAvgBytesPerSec: rate * u32::from(channels) * 2,
                    nBlockAlign: channels * 2,
                    wBitsPerSample: 16,
                    cbSize: 0,
                };
                result._engine.as_ref().unwrap().CreateSourceVoice(
                    &mut result.voice,
                    &format,
                    0,
                    1.,
                    None,
                    None,
                    None,
                )?;
            }
        }
        Ok(result)
    }
    pub fn playing(&mut self, playing: bool) -> Result<()> {
        self.position();
        if let Some(voice) = &self.voice {
            unsafe {
                if playing {
                    voice.Start(0, 0)?;
                } else {
                    voice.Stop(0, 0)?;
                }
            }
        }
        self.playing = playing;
        self.updated = Instant::now();
        Ok(())
    }
    pub fn volume(&self, value: f64) -> Result<()> {
        if let Some(voice) = &self.voice {
            unsafe {
                voice.SetVolume(value as f32, 0)?;
            }
        }
        Ok(())
    }
    pub fn submit(&mut self, pcm: Vec<i16>) -> Result<()> {
        if let Some(voice) = &self.voice {
            let buffer = XAUDIO2_BUFFER {
                AudioBytes: pcm
                    .len()
                    .checked_mul(2)
                    .and_then(|n| n.try_into().ok())
                    .context("Audio buffer too large")?,
                pAudioData: pcm.as_ptr().cast(),
                ..Default::default()
            };
            unsafe {
                voice.SubmitSourceBuffer(&buffer, None)?;
            }
        }
        self.submitted_frames += (pcm.len() / self.channels) as u64;
        self.buffers.push_back(pcm);
        Ok(())
    }
    pub fn position(&mut self) -> u64 {
        if let Some(voice) = &self.voice {
            let mut state = XAUDIO2_VOICE_STATE::default();
            unsafe {
                voice.GetState(&mut state, 0);
            }
            while self.buffers.len() > state.BuffersQueued as usize {
                self.buffers.pop_front();
            }
            state.SamplesPlayed
        } else {
            if self.playing {
                let samples = self.updated.elapsed().as_secs_f64() * f64::from(self.rate)
                    + self.fake_fraction;
                self.fake_position =
                    (self.fake_position + samples as u64).min(self.submitted_frames);
                self.fake_fraction = samples.fract();
            }
            self.updated = Instant::now();
            let mut pending = self.submitted_frames - self.fake_position;
            let mut keep = 0;
            for buffer in self.buffers.iter().rev() {
                if pending == 0 {
                    break;
                }
                pending = pending.saturating_sub((buffer.len() / self.channels) as u64);
                keep += 1;
            }
            while self.buffers.len() > keep {
                self.buffers.pop_front();
            }
            self.fake_position
        }
    }
    pub fn queued(&self) -> usize {
        self.buffers.len()
    }
    pub fn queued_frames(&self, played: u64) -> u64 {
        self.submitted_frames.saturating_sub(played)
    }
}
impl Drop for Output {
    fn drop(&mut self) {
        // DestroyVoice waits for the processing thread before freeing borrowed PCM.
        unsafe {
            if let Some(voice) = self.voice.take() {
                voice.DestroyVoice();
            }
            if let Some(master) = self.master.take() {
                master.DestroyVoice();
            }
        }
    }
}
