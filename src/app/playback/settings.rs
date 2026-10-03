use crate::app::{Command, Controller, Event, audio::AudioCommand, text};
use anyhow::Result;

impl Controller {
    pub(in crate::app) fn set_volume(&mut self, volume: f64, muted: bool) -> Result<()> {
        if !volume.is_finite() {
            return Ok(());
        }
        let volume = volume.clamp(0., 1.);
        if self.library.session.volume == volume && self.library.session.muted == muted {
            return Ok(());
        }
        self.library.session.volume = volume;
        self.library.session.muted = muted;
        self.audio.send(AudioCommand::Volume(volume, muted));
        // A volume drag changes no book metadata. Publish the applied level
        // even if its checkpoint fails, so controls still match audible output.
        let _ = self.events.try_send(Event::Volume(volume, muted));
        self.save()
    }
    pub(in crate::app) fn set_rate(&mut self, rate: f64) -> Result<()> {
        anyhow::ensure!(
            rate.is_finite() && (crate::player::MIN_RATE..=crate::player::MAX_RATE).contains(&rate),
            text(
                "Скорость должна быть от 0,5× до 3×",
                "Speed must be between 0.5× and 3×"
            )
        );
        self.token = self.token.wrapping_add(1);
        self.playback.token = self.token;
        self.requested_rate = rate;
        if self.options.demo {
            self.playback.rate = rate;
            return self
                .handle(Command::Playback(self.playback.clone()))
                .map(|_| ());
        }
        self.audio.send(AudioCommand::Rate(self.token, rate));
        Ok(())
    }
    pub(in crate::app) fn step_rate(&mut self, forward: bool) -> Result<()> {
        if self.options.demo {
            return self
                .handle(Command::Rate(crate::player::stepped_rate(
                    self.playback.rate,
                    forward,
                )))
                .map(|_| ());
        }
        self.token = self.token.wrapping_add(1);
        self.playback.token = self.token;
        self.requested_rate = crate::player::stepped_rate(self.requested_rate, forward);
        self.audio.send(AudioCommand::RateStep(self.token, forward));
        Ok(())
    }
    pub(in crate::app) fn set_skip_silence(&mut self, enabled: bool) -> Result<()> {
        self.token = self.token.wrapping_add(1);
        self.playback.token = self.token;
        self.requested_skip_silence = enabled;
        if self.options.demo {
            self.playback.skip_silence = enabled;
            return self
                .handle(Command::Playback(self.playback.clone()))
                .map(|_| ());
        }
        self.audio
            .send(AudioCommand::SkipSilence(self.token, enabled));
        Ok(())
    }
}
