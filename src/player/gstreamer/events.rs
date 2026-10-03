use super::{Event, EventKind, Player};
use crate::player::Phase;
use anyhow::Result;
use gst::prelude::*;

impl Player {
    /// Returns true only when a seek/restore has completed, never for events from an old pipeline.
    pub fn handle(&mut self, event: &Event) -> Result<bool> {
        if event.generation != self.generation {
            return Ok(false);
        }
        // A completion or EOS from before this seek belongs to the old position,
        // even when delivered after the new seek has completed.
        if let EventKind::AsyncDone(seqnum) | EventKind::Eos(seqnum) = event.kind
            && self.seek_seqnum.is_some_and(|start| seqnum < start)
        {
            return Ok(false);
        }
        match event.kind {
            EventKind::AsyncDone(_) if self.phase == Phase::Loading => {
                self.query_duration();
                let mut query = gst::query::Seeking::new(gst::Format::Time);
                self.seekable =
                    self.pipeline.as_ref().is_some_and(|p| p.query(&mut query)) && query.result().0;
                let target = self.requested_seek.take().unwrap_or(0);
                if (target > 0 || self.rate != 1.0) && self.seekable {
                    self.start_seek(target)?;
                    return Ok(false);
                }
                if target > 0 && !self.seekable {
                    self.phase = Phase::Error;
                    self.playing = false;
                    anyhow::bail!(crate::i18n::tr(
                        "Файл не поддерживает восстановление позиции"
                    ));
                }
                if self.rate != 1.0 && !self.seekable {
                    self.rate = 1.0;
                }
                self.phase = Phase::Ready;
                self.poll();
                self.apply_state()?;
            }
            EventKind::AsyncDone(_) if self.phase == Phase::Seeking => {
                if let Some(target) = self.requested_seek.take() {
                    self.start_seek(target)?;
                    return Ok(false);
                }
                self.phase = Phase::Ready;
                self.inflight_seek = None;
                self.poll();
                self.apply_state()?;
                return Ok(true);
            }
            EventKind::Duration => self.query_duration(),
            EventKind::Eos(_) if !matches!(self.phase, Phase::Error | Phase::Empty) => {
                self.poll();
                self.position = self.duration.unwrap_or(self.position);
                self.ended = true;
            }
            EventKind::Error(_) => {
                self.fail();
            }
            _ => {}
        }
        Ok(false)
    }
    pub(super) fn fail(&mut self) {
        // Requesting Paused again after a failed load restarts playbin's source
        // and produces another error, which would re-enter this method forever.
        if self.phase == Phase::Error {
            return;
        }
        self.poll();
        self.playing = false;
        self.seekable = false;
        self.phase = Phase::Error;
        if let Some(p) = &self.pipeline {
            let _ = p.set_state(gst::State::Paused);
        }
    }
    fn query_duration(&mut self) {
        if let Some(p) = &self.pipeline {
            self.duration = p.query_duration::<gst::ClockTime>().map(|t| t.mseconds());
        }
    }
    pub fn poll(&mut self) {
        if self.phase == Phase::Ready
            && !self.ended
            && let Some(pos) = self
                .pipeline
                .as_ref()
                .and_then(|p| p.query_position::<gst::ClockTime>())
        {
            self.position = self
                .silence
                .as_ref()
                .map_or(pos, |s| s.source_position(pos))
                .mseconds();
        }
    }
}
