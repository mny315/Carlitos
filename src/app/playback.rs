mod events;
mod settings;

use super::{
    Controller, Event,
    audio::{AudioCommand, Playback},
    text,
};
use crate::library::*;
use anyhow::{Context, Result};

impl Controller {
    pub(super) fn load(&mut self, id: Id, position: u64, playing: bool) -> Result<()> {
        let uri = self
            .library
            .file_for(&Target::Book(id))
            .context(text("Часть не найдена", "Part not found"))?
            .uri
            .clone();
        if self.options.demo {
            self.notice(
                text(
                    "Демонстрация оформления. Добавьте свои книги в обычном режиме.",
                    "Design preview. Add your books in normal mode.",
                )
                .into(),
            );
            return Ok(());
        }
        anyhow::ensure!(
            !self.audio.is_closed(),
            text(
                "Аудиодвижок недоступен. Перезапустите приложение.",
                "Audio is unavailable. Restart the application."
            )
        );
        if self.library.session.current != Some(Target::Book(id)) {
            self.save()?;
            self.library.session.position = self
                .library
                .progress
                .iter()
                .find(|progress| progress.part_id == id && !progress.completed)
                .map_or(0, |progress| progress.position);
        }
        self.library.session.current = Some(Target::Book(id));
        // The requested chapter/seek position belongs to Playback until Ready.
        // A missing source must not replace the last confirmed checkpoint.
        // Persist a new selection as soon as the decoder confirms its position,
        // including a paused part change before Android kills the process.
        self.pending_save = true;
        self.token = self.token.wrapping_add(1);
        self.playback = Playback {
            token: self.token,
            position,
            playing,
            rate: self.requested_rate,
            skip_silence: self.requested_skip_silence,
            phase: crate::player::Phase::Loading,
            ..Default::default()
        };
        self.audio.send(AudioCommand::Load {
            token: self.token,
            uri,
            position,
            playing,
            #[cfg(target_os = "android")]
            title: self
                .library
                .part(id)
                .map(|part| part.title.clone())
                .unwrap_or_default(),
            #[cfg(target_os = "android")]
            rate: self.requested_rate,
            #[cfg(target_os = "android")]
            skip_silence: self.requested_skip_silence,
        });
        self.audio.send(AudioCommand::Volume(
            self.library.session.volume,
            self.library.session.muted,
        ));
        self.emit_library();
        Ok(())
    }
    pub(super) fn active_book(&self) -> Option<Id> {
        let Target::Book(id) = self.library.session.current.as_ref()?;
        self.library.part(*id).map(|p| p.book_id)
    }
    pub(super) fn play(&mut self, value: bool) -> Result<()> {
        let Some(Target::Book(id)) = self.library.session.current else {
            return Ok(());
        };
        if value
            && let Some(book) = self.active_book()
            && self
                .library
                .progress
                .iter()
                .any(|p| p.book_id == book && p.completed)
            && let Some(first) = self.library.book_parts(book).first()
        {
            return self.load(first.id, 0, true);
        }
        if value && self.playback.phase == crate::player::Phase::Error {
            return self.load(id, self.library.session.position, true);
        }
        self.token = self.token.wrapping_add(1);
        self.playback.token = self.token;
        self.playback.playing = value;
        self.audio.send(AudioCommand::Playing(self.token, value));
        if !value {
            self.pending_save = true;
        }
        Ok(())
    }
    pub(super) fn seek_to(&mut self, position: u64) {
        if !self.playback.seekable
            && !matches!(
                self.playback.phase,
                crate::player::Phase::Loading | crate::player::Phase::Seeking
            )
        {
            return;
        }
        self.token = self.token.wrapping_add(1);
        self.playback.token = self.token;
        self.playback.position = position;
        self.playback.phase = crate::player::Phase::Seeking;
        self.audio.send(AudioCommand::Seek(self.token, position));
    }
    pub(super) fn stop(&mut self) {
        self.token = self.token.wrapping_add(1);
        self.audio.send(AudioCommand::Stop(self.token));
        self.library.session.current = None;
        self.library.session.position = 0;
        self.playback = Playback {
            token: self.token,
            rate: self.options.settings.playback_rate,
            skip_silence: self.options.settings.skip_silence,
            ..Default::default()
        };
        let _ = self.events.try_send(Event::Playback(self.playback.clone()));
    }
    pub(super) fn resume(&mut self, book: Id) -> Result<()> {
        let completed = self
            .library
            .progress
            .iter()
            .any(|p| p.book_id == book && p.completed);
        if self.active_book() == Some(book) && !completed {
            self.play(!self.playback.playing)?;
        } else {
            let saved = self
                .library
                .progress
                .iter()
                .find(|p| p.book_id == book && !p.completed);
            let (part, position) = saved
                .map(|p| (p.part_id, p.position))
                .or_else(|| self.library.book_parts(book).first().map(|p| (p.id, 0)))
                .context(text("В книге нет частей", "This book has no parts"))?;
            self.load(part, position, true)?;
        }
        Ok(())
    }
    pub(super) fn seek_delta(&mut self, delta: i64) -> Result<()> {
        let seq = self.library.sequence();
        if let Some(index) = seq
            .iter()
            .position(|t| Some(t) == self.library.session.current.as_ref())
        {
            let durations: Vec<_> = seq
                .iter()
                .map(|t| self.library.file_for(t).and_then(|f| f.duration))
                .collect();
            let (next, position, _) = book_seek(&durations, index, self.playback.position, delta);
            if next != index {
                let Target::Book(id) = seq[next];
                self.load(id, position, self.playback.playing)?;
            } else {
                self.seek_to(position);
            }
        }
        Ok(())
    }
}
