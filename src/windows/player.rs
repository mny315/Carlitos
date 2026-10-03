use crate::player::{MAX_RATE, MIN_RATE, Phase};
use crate::{
    audio_processing::Timeline,
    library::Millis,
    windows::{Apartment, decoder::Decoder, output::Output, tempo::Tempo},
};
use anyhow::{Context, Result, ensure};

#[derive(Clone, Debug)]
pub enum EventKind {
    AsyncDone(u64),
    Error(String),
}
#[derive(Clone, Debug)]
pub struct Event {
    pub generation: u64,
    pub kind: EventKind,
}
pub struct Player {
    output: Option<Output>,
    decoder: Option<Decoder>,
    tempo: Option<Tempo>,
    timeline: Timeline,
    sender: async_channel::Sender<Event>,
    pub generation: u64,
    pub phase: Phase,
    pub playing: bool,
    pub ended: bool,
    pub position: Millis,
    pub duration: Option<Millis>,
    pub seekable: bool,
    base: Millis,
    revision: u64,
    volume: f64,
    muted: bool,
    rate: f64,
    skip_silence: bool,
    flushed: bool,
    fake: bool,
    _apartment: Apartment,
}
impl Player {
    pub fn new(sender: async_channel::Sender<Event>, fake: bool) -> Result<Self> {
        let apartment = Apartment::new()?;
        crate::windows::initialize_media()?;
        Ok(Self {
            output: None,
            decoder: None,
            tempo: None,
            timeline: Timeline::default(),
            sender,
            generation: 0,
            phase: Phase::Empty,
            playing: false,
            ended: false,
            position: 0,
            duration: None,
            seekable: false,
            base: 0,
            revision: 0,
            volume: 0.7,
            muted: false,
            rate: 1.,
            skip_silence: false,
            flushed: false,
            fake,
            _apartment: apartment,
        })
    }
    pub fn load(&mut self, uri: &str, position: Millis, playing: bool) -> Result<()> {
        self.stop();
        self.phase = Phase::Error;
        let decoder = Decoder::open(uri)?;
        self.duration = decoder.duration;
        self.decoder = Some(decoder);
        self.playing = playing;
        self.seekable = true;
        self.seek(position)
    }
    pub fn handle(&mut self, event: &Event) -> Result<bool> {
        if event.generation != self.generation {
            return Ok(false);
        }
        match &event.kind {
            EventKind::AsyncDone(revision) => {
                Ok(*revision == self.revision && self.phase == Phase::Ready)
            }
            EventKind::Error(_) => {
                self.fail();
                Ok(false)
            }
        }
    }
    fn fail(&mut self) {
        self.output = None;
        self.phase = Phase::Error;
        self.playing = false;
        self.seekable = false;
        self.ended = false;
    }
    pub fn poll(&mut self) {
        if let Err(e) = self.pump() {
            self.fail();
            let _ = self.sender.try_send(Event {
                generation: self.generation,
                kind: EventKind::Error(format!("{e:#}")),
            });
        }
    }
    fn pump(&mut self) -> Result<()> {
        if self.phase != Phase::Ready || self.ended {
            return Ok(());
        }
        let output = self.output.as_mut().context("Audio output missing")?;
        let decoder = self.decoder.as_mut().context("Audio decoder missing")?;
        let played = output.position();
        let source = self
            .timeline
            .source_frame((played as f64 * self.rate) as u64);
        self.position = self
            .base
            .saturating_add(source.saturating_mul(1000) / u64::from(decoder.rate));
        if let Some(duration) = self.duration {
            self.position = self.position.min(duration);
        }
        if !self.playing || self.ended {
            return Ok(());
        }
        let tempo = self.tempo.as_mut().context("Audio processor missing")?;
        // Bound decoding per tick, including a file containing hours of silence.
        for _ in 0..64 {
            if output.queued_frames(played) >= u64::from(decoder.rate) / 3 || output.queued() >= 60
            {
                break;
            }
            let pcm = tempo.read((decoder.rate / 10) as usize);
            if !pcm.is_empty() {
                output.submit(pcm)?;
                continue;
            }
            if decoder.end {
                if !self.flushed {
                    tempo.finish()?;
                    self.flushed = true;
                    continue;
                }
                if output.queued() == 0 {
                    self.position = self.duration.unwrap_or(self.position);
                    self.ended = true;
                }
                break;
            }
            let pcm = decoder.read()?;
            let pcm = self.timeline.filter(
                &pcm,
                usize::from(decoder.channels),
                decoder.rate,
                self.skip_silence,
            );
            if !pcm.is_empty() {
                tempo.write(&pcm)?;
            }
        }
        Ok(())
    }
    pub fn seek(&mut self, position: Millis) -> Result<()> {
        if self.decoder.is_none() {
            return Ok(());
        }
        let result = self.reset(position);
        if result.is_err() {
            self.fail();
        }
        result
    }
    fn reset(&mut self, position: Millis) -> Result<()> {
        // A seek replaces the output and its queued completion/error events.
        // The worker may receive an old error only after this reset succeeds.
        self.generation = self.generation.wrapping_add(1);
        self.phase = Phase::Seeking;
        self.output = None;
        self.tempo = None;
        let decoder = self.decoder.as_mut().unwrap();
        let position = position
            .min(self.duration.unwrap_or(u64::MAX))
            .min(i64::MAX as u64 / 10_000);
        decoder.seek(position)?;
        let mut output = Output::new(decoder.rate, decoder.channels, self.fake)?;
        output.volume(if self.muted { 0. } else { self.volume })?;
        output.playing(self.playing)?;
        self.tempo = Some(Tempo::new(decoder.rate, decoder.channels, self.rate)?);
        self.output = Some(output);
        self.timeline = Timeline::default();
        self.base = position;
        self.position = position;
        self.ended = false;
        self.flushed = false;
        self.seekable = true;
        self.phase = Phase::Ready;
        self.revision = self.revision.wrapping_add(1);
        let _ = self.sender.try_send(Event {
            generation: self.generation,
            kind: EventKind::AsyncDone(self.revision),
        });
        Ok(())
    }
    pub fn seek_base(&self) -> Millis {
        self.position
    }
    pub fn rate(&self) -> f64 {
        self.rate
    }
    pub fn skip_silence(&self) -> bool {
        self.skip_silence
    }
    pub fn take_silence_skip(&mut self) -> bool {
        std::mem::take(&mut self.timeline.discontinuity)
    }
    pub fn set_rate(&mut self, rate: f64) -> Result<()> {
        ensure!(
            rate.is_finite() && (MIN_RATE..=MAX_RATE).contains(&rate),
            crate::i18n::tr("Скорость должна быть от 0,5× до 3×")
        );
        if rate == self.rate {
            return Ok(());
        }
        self.poll();
        self.rate = rate;
        self.seek(self.position)
    }
    pub fn set_skip_silence(&mut self, enabled: bool) -> Result<()> {
        if enabled == self.skip_silence {
            return Ok(());
        }
        self.poll();
        self.skip_silence = enabled;
        self.seek(self.position)
    }
    pub fn set_playing(&mut self, playing: bool) -> Result<()> {
        if matches!(self.phase, Phase::Empty | Phase::Error) {
            return Ok(());
        }
        if playing && self.ended {
            self.seek(0)?;
        }
        if let Some(output) = &mut self.output
            && let Err(error) = output.playing(playing)
        {
            // The controller retries a failed device by reloading the file.
            // Do not leave a failed Start/Stop marked as a usable output.
            self.fail();
            return Err(error);
        }
        self.playing = playing;
        Ok(())
    }
    pub fn volume(&mut self, volume: f64, muted: bool) {
        if volume.is_finite() {
            self.volume = volume.clamp(0., 1.);
        }
        self.muted = muted;
        if let Some(output) = &self.output
            && let Err(error) = output.volume(if muted { 0. } else { self.volume })
        {
            let _ = self.sender.try_send(Event {
                generation: self.generation,
                kind: EventKind::Error(error.to_string()),
            });
        }
    }
    pub fn stop(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.output = None;
        self.decoder = None;
        self.tempo = None;
        self.phase = Phase::Empty;
        self.playing = false;
        self.ended = false;
        self.position = 0;
        self.duration = None;
        self.seekable = false;
    }
}
