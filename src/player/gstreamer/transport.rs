use super::Player;
use crate::{
    library::Millis,
    player::{MAX_RATE, MIN_RATE, Phase},
};
use anyhow::{Context, Result};
use gst::prelude::*;

impl Player {
    pub fn seek(&mut self, target: Millis) -> Result<()> {
        if self.phase == Phase::Loading || self.phase == Phase::Seeking {
            self.requested_seek = Some(target);
            return Ok(());
        }
        if self.phase != Phase::Ready || !self.seekable {
            return Ok(());
        }
        self.start_seek(target)
    }
    pub(super) fn start_seek(&mut self, target: Millis) -> Result<()> {
        // ClockTime stores nanoseconds, so not every u64 millisecond value is
        // representable (for example a malformed saved session or MPRIS seek).
        let target = target
            .min(self.duration.unwrap_or(target))
            .min(gst::ClockTime::MAX.mseconds());
        let seqnum = gst::Seqnum::next();
        let result = self
            .pipeline
            .as_ref()
            .context(crate::i18n::tr("Нет активного файла"))
            .and_then(|pipeline| {
                pipeline
                    .seek(
                        self.rate,
                        gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                        gst::SeekType::Set,
                        gst::ClockTime::from_mseconds(target),
                        gst::SeekType::None,
                        gst::ClockTime::NONE,
                    )
                    .map_err(Into::into)
            });
        if let Err(error) = result {
            if matches!(self.phase, Phase::Loading | Phase::Seeking) {
                // There will be no AsyncDone for a rejected restore/queued
                // seek. Leave the transient phase so playback can be retried.
                self.fail();
            }
            return Err(error);
        }
        self.phase = Phase::Seeking;
        self.seek_seqnum = Some(seqnum);
        self.inflight_seek = Some(target);
        self.ended = false;
        Ok(())
    }
    pub fn seek_base(&self) -> Millis {
        self.requested_seek
            .or(self.inflight_seek)
            .unwrap_or(self.position)
    }
    pub fn rate(&self) -> f64 {
        self.rate
    }
    pub fn skip_silence(&self) -> bool {
        self.skip_silence
    }
    /// A source-position jump needs the same checkpoint and MPRIS Seeked signal as a seek.
    pub fn take_silence_skip(&self) -> bool {
        self.silence
            .as_ref()
            .is_some_and(|s| s.take_discontinuity())
    }
    pub fn set_skip_silence(&mut self, enabled: bool) -> Result<()> {
        if self.skip_silence == enabled {
            return Ok(());
        }
        anyhow::ensure!(
            self.phase != Phase::Ready || self.seekable,
            crate::i18n::tr("Файл не поддерживает пропуск тишины")
        );
        self.poll();
        let position = self.seek_base();
        let previous = self.skip_silence;
        self.skip_silence = enabled;
        if let Some(silence) = &self.silence {
            silence.set_enabled(enabled);
        }
        // Flush already queued audio and reset the time map at the same source position.
        if let Err(error) = self.seek(position) {
            self.skip_silence = previous;
            if let Some(silence) = &self.silence {
                silence.set_enabled(previous);
            }
            return Err(error);
        }
        Ok(())
    }
    pub fn set_rate(&mut self, rate: f64) -> Result<()> {
        anyhow::ensure!(
            rate.is_finite() && (MIN_RATE..=MAX_RATE).contains(&rate),
            crate::i18n::tr("Скорость должна быть от 0,5× до 3×")
        );
        if rate == self.rate {
            return Ok(());
        }
        anyhow::ensure!(
            self.phase != Phase::Ready || self.seekable,
            crate::i18n::tr("Файл не поддерживает изменение скорости")
        );
        self.poll();
        let previous = self.rate;
        self.rate = rate;
        // Queue changes behind an in-flight seek without losing its target.
        if let Err(error) = self.seek(self.seek_base()) {
            self.rate = previous;
            return Err(error);
        }
        Ok(())
    }
    pub fn set_playing(&mut self, playing: bool) -> Result<()> {
        if matches!(self.phase, Phase::Empty | Phase::Error) {
            return Ok(());
        }
        self.playing = playing;
        if playing && self.ended && self.seekable {
            self.start_seek(0)?;
        }
        if self.phase == Phase::Ready {
            self.apply_state()?;
        }
        Ok(())
    }
    pub(super) fn apply_state(&mut self) -> Result<()> {
        if let Some(p) = &self.pipeline
            && let Err(error) = p.set_state(if self.playing {
                gst::State::Playing
            } else {
                gst::State::Paused
            })
        {
            // A synchronous state-change failure need not wait for a bus
            // error before the controller can offer a fresh load on retry.
            self.fail();
            return Err(error.into());
        }
        Ok(())
    }
    pub fn volume(&mut self, volume: f64, muted: bool) {
        if volume.is_finite() {
            self.volume = volume.clamp(0., 1.);
        }
        self.muted = muted;
        if let Some(p) = &self.pipeline {
            p.set_property("volume", self.volume);
            p.set_property("mute", muted);
        }
    }
    pub fn stop(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.watch.take();
        if let Some(p) = self.pipeline.take() {
            let _ = p.set_state(gst::State::Null);
        }
        self.silence = None;
        self.phase = Phase::Empty;
        self.playing = false;
        self.position = 0;
        self.duration = None;
        self.seekable = false;
        self.requested_seek = None;
        self.inflight_seek = None;
        self.seek_seqnum = None;
        self.ended = false;
    }
}
