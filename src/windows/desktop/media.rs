use crate::app::{Command, audio::Playback};
use carlitos::library::{Library, Target};
use std::sync::{Arc, Mutex, mpsc::Sender};
use windows::{
    Foundation::TypedEventHandler,
    Media::*,
    Win32::{Foundation::HWND, System::WinRT::ISystemMediaTransportControlsInterop},
    core::HSTRING,
};

pub(super) struct MediaControls {
    controls: SystemMediaTransportControls,
    button_token: Option<i64>,
    position_token: Option<i64>,
    rate_token: Option<i64>,
    selection: Arc<Mutex<Option<(i64, String)>>>,
}
impl MediaControls {
    pub(super) fn new(hwnd: HWND, commands: Sender<Command>) -> windows::core::Result<Self> {
        let factory: ISystemMediaTransportControlsInterop =
            windows::core::factory::<SystemMediaTransportControls, _>()?;
        let controls: SystemMediaTransportControls = unsafe { factory.GetForWindow(hwnd)? };
        // Own registrations immediately, so a later initialization failure also
        // revokes callbacks and disables the partially configured media session.
        let mut result = Self {
            controls,
            button_token: None,
            position_token: None,
            rate_token: None,
            selection: Arc::default(),
        };
        result.controls.SetIsPlayEnabled(true)?;
        result.controls.SetIsPauseEnabled(true)?;
        result.controls.SetIsStopEnabled(true)?;
        let position_commands = commands.clone();
        let rate_commands = commands.clone();
        result.button_token =
            Some(result.controls.ButtonPressed(&TypedEventHandler::<
                SystemMediaTransportControls,
                SystemMediaTransportControlsButtonPressedEventArgs,
            >::new(move |_, args| {
                if let Some(args) = args.as_ref() {
                    let command = match args.Button()? {
                        SystemMediaTransportControlsButton::Play => Some(Command::Playing(true)),
                        SystemMediaTransportControlsButton::Pause => Some(Command::Playing(false)),
                        SystemMediaTransportControlsButton::Stop => Some(Command::Stop),
                        SystemMediaTransportControlsButton::Next => Some(Command::Next(true)),
                        SystemMediaTransportControlsButton::Previous => Some(Command::Next(false)),
                        _ => None,
                    };
                    if let Some(command) = command {
                        let _ = commands.send(command);
                    }
                }
                Ok(())
            }))?);
        let selected = result.selection.clone();
        result.position_token = Some(result.controls.PlaybackPositionChangeRequested(
            &TypedEventHandler::<
                SystemMediaTransportControls,
                PlaybackPositionChangeRequestedEventArgs,
            >::new(move |_, args| {
                if let Some(args) = args.as_ref() {
                    let position = args.RequestedPlaybackPosition()?.Duration;
                    if position >= 0
                        && let Some((id, uri)) = selected.lock().unwrap().clone()
                    {
                        let _ = position_commands.send(Command::SetPosition(
                            id,
                            uri,
                            position as u64 / 10_000,
                        ));
                    }
                }
                Ok(())
            }),
        )?);
        result.rate_token = Some(
            result
                .controls
                .PlaybackRateChangeRequested(&TypedEventHandler::<
                    SystemMediaTransportControls,
                    PlaybackRateChangeRequestedEventArgs,
                >::new(move |_, args| {
                    if let Some(args) = args.as_ref() {
                        let _ = rate_commands.send(Command::Rate(args.RequestedPlaybackRate()?));
                    }
                    Ok(())
                }))?,
        );
        result.controls.SetIsEnabled(true)?;
        Ok(result)
    }
    pub(super) fn library(&self, library: &Library) -> windows::core::Result<()> {
        let updater = self.controls.DisplayUpdater()?;
        updater.ClearAll()?;
        updater.SetType(MediaPlaybackType::Music)?;
        *self.selection.lock().unwrap() = None;
        if let Some(Target::Book(id)) = library.session.current
            && let Some(part) = library.part(id)
            && let Some(book) = library.books.iter().find(|b| b.id == part.book_id)
        {
            let music = updater.MusicProperties()?;
            music.SetTitle(&HSTRING::from(&book.title))?;
            music.SetArtist(&HSTRING::from(&book.author))?;
            if let Some(file) = library.media(part.file_id) {
                *self.selection.lock().unwrap() = Some((id, file.uri.clone()));
            }
            if let Some(cover) = &book.cover
                && let Ok(uri) = carlitos::library::file_uri(std::path::Path::new(cover))
                && let Ok(uri) = windows::Foundation::Uri::CreateUri(&HSTRING::from(uri))
                && let Ok(image) =
                    windows::Storage::Streams::RandomAccessStreamReference::CreateFromUri(&uri)
            {
                let _ = updater.SetThumbnail(&image);
            }
        }
        self.controls
            .SetIsNextEnabled(library.neighbour(true).is_some())?;
        self.controls
            .SetIsPreviousEnabled(library.neighbour(false).is_some())?;
        updater.Update()
    }
    pub(super) fn playback(&self, playback: &Playback) -> windows::core::Result<()> {
        self.controls
            .SetPlaybackStatus(if playback.phase == carlitos::player::Phase::Empty {
                MediaPlaybackStatus::Stopped
            } else if playback.playing {
                MediaPlaybackStatus::Playing
            } else {
                MediaPlaybackStatus::Paused
            })?;
        self.controls.SetPlaybackRate(playback.rate)?;
        let timeline = SystemMediaTransportControlsTimelineProperties::new()?;
        let time = |millis: u64| windows::Foundation::TimeSpan {
            Duration: millis.min(i64::MAX as u64 / 10_000) as i64 * 10_000,
        };
        timeline.SetStartTime(time(0))?;
        timeline.SetEndTime(time(playback.duration.unwrap_or(playback.position)))?;
        timeline.SetMinSeekTime(time(0))?;
        timeline.SetMaxSeekTime(time(if playback.seekable {
            playback.duration.unwrap_or(0)
        } else {
            0
        }))?;
        timeline.SetPosition(time(playback.position))?;
        self.controls.UpdateTimelineProperties(&timeline)
    }
}
impl Drop for MediaControls {
    fn drop(&mut self) {
        if let Some(token) = self.button_token {
            let _ = self.controls.RemoveButtonPressed(token);
        }
        if let Some(token) = self.position_token {
            let _ = self.controls.RemovePlaybackPositionChangeRequested(token);
        }
        if let Some(token) = self.rate_token {
            let _ = self.controls.RemovePlaybackRateChangeRequested(token);
        }
        let _ = self.controls.SetIsEnabled(false);
    }
}
