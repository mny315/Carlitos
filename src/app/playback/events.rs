use crate::{
    app::{Controller, Event, audio::Playback},
    library::Target,
};
use anyhow::Result;
use std::time::Duration;

impl Controller {
    pub(in crate::app) fn receive_audio_failure(
        &mut self,
        token: u64,
        message: String,
    ) -> Result<bool> {
        self.receive_playback(Playback {
            token,
            position: self.library.session.position,
            playing: false,
            phase: crate::player::Phase::Error,
            seekable: false,
            seek_done: false,
            ended: false,
            // Transport failures must also release a pending shutdown barrier.
            barrier: true,
            error: Some(message),
            ..self.playback.clone()
        })
    }
    pub(in crate::app) fn receive_playback(&mut self, snapshot: Playback) -> Result<bool> {
        if snapshot.token != self.token {
            return Ok(false);
        }
        let was_playing = self.playback.playing;
        let previous_position = self.playback.position;
        let was_ended = self.playback.ended;
        let error_changed = self.playback.error != snapshot.error;
        self.playback = snapshot.clone();
        // A failed checkpoint must not acknowledge EOS permanently.
        // Load resets the latch; the final-part branch commits it only
        // after its progress has actually been saved.
        self.playback.ended = was_ended && snapshot.ended;
        if matches!(
            snapshot.phase,
            crate::player::Phase::Ready | crate::player::Phase::Empty
        ) {
            self.requested_rate = snapshot.rate;
            self.requested_skip_silence = snapshot.skip_silence;
        }
        if matches!(
            snapshot.phase,
            crate::player::Phase::Ready | crate::player::Phase::Empty
        ) && (self.options.settings.playback_rate != snapshot.rate
            || self.options.settings.skip_silence != snapshot.skip_silence)
        {
            self.options.settings.playback_rate = snapshot.rate;
            self.options.settings.skip_silence = snapshot.skip_silence;
            if self.options.settings_writable
                && !self.options.demo
                && let Err(error) = self.options.settings.save(&self.options.settings_path)
            {
                // The audio change already happened. Report the failed
                // preference write without dropping this playback event.
                self.notice(format!("{error:#}"));
            }
            let _ = self
                .events
                .try_send(Event::Settings(self.options.settings.clone()));
        }
        if snapshot.phase == crate::player::Phase::Ready {
            let completed = self.active_book().is_some_and(|book| {
                self.library
                    .progress
                    .iter()
                    .any(|p| p.book_id == book && p.completed)
            }) && !snapshot.playing
                // EOS is authoritative even when the decoder has no duration.
                && (snapshot.ended
                    || snapshot
                        .duration
                        .is_some_and(|duration| snapshot.position >= duration));
            self.library.update_progress(snapshot.position, completed);
            if let Some(duration) = snapshot.duration
                && let Some(Target::Book(id)) = self.library.session.current.clone()
                && let Some(file_id) = self.library.part(id).map(|p| p.file_id)
                && let Some(file) = self.library.media.iter_mut().find(|f| f.id == file_id)
            {
                file.duration = Some(duration);
            }
            if self.pending_save
                || snapshot.seek_done
                || (was_playing && !snapshot.playing)
                || (!snapshot.playing && snapshot.position != previous_position)
                || self.checkpoint.elapsed() >= Duration::from_secs(4)
            {
                self.save()?;
                self.pending_save = false;
            }
        }
        if error_changed && let Some(error) = snapshot.error.as_ref() {
            // Error positions can be zero or an unconfirmed seek target. Keep
            // the last Ready position, but flush it before the process can die.
            // Startup may already have failed to open the database. Its empty
            // fallback has no checkpoint to write and must still allow Quit.
            if self.store.as_ref().is_some_and(|store| store.is_ok()) {
                self.save()?;
            }
            self.notice(error.clone());
        }
        let _ = self.events.try_send(Event::Playback(snapshot.clone()));
        if self.quitting && snapshot.barrier {
            let session = self.library.session.clone();
            let progress = self.library.progress.clone();
            if snapshot.ended {
                if let Some(next) = self.library.neighbour(true) {
                    self.library.session.current = Some(next);
                    self.library.update_progress(0, false);
                } else {
                    self.library.update_progress(snapshot.position, true);
                }
            }
            if let Err(error) = self.finish() {
                // Shutdown may be cancelled by a failed write. The
                // audio worker still owns the old part and may resend
                // its EOS, so retain its matching in-memory session.
                self.library.session = session;
                self.library.progress = progress;
                return Err(error);
            }
            return Ok(true);
        }
        if snapshot.ended && !was_ended {
            if let Some(Target::Book(next)) = self.library.neighbour(true) {
                self.load(next, 0, snapshot.playing)?;
            } else {
                self.library
                    .update_progress(snapshot.duration.unwrap_or(snapshot.position), true);
                self.play(false)?;
                self.save()?;
                self.playback.ended = true;
                self.emit_library();
            }
        }
        Ok(false)
    }
}
