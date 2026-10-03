use crate::app::Command;
use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::{Arc, RwLock, mpsc::Sender},
};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Str, Value};

pub(super) const NAME: &str = "org.mpris.MediaPlayer2.Carlitos";
pub(super) const PATH: &str = "/org/mpris/MediaPlayer2";

#[derive(Clone, Default)]
pub(super) struct MediaState {
    pub(super) title: String,
    pub(super) author: String,
    pub(super) uri: String,
    pub(super) art: String,
    pub(super) track: Option<i64>,
    pub(super) duration: Option<u64>,
    pub(super) position: u64,
    pub(super) playing: bool,
    pub(super) seekable: bool,
    pub(super) next: bool,
    pub(super) previous: bool,
    pub(super) volume: f64,
    pub(super) rate: f64,
}
impl MediaState {
    pub(super) fn track_path(&self) -> String {
        self.track
            .map(|id| {
                // SQLite can reuse a deleted part's ID for a different file.
                let mut hash = std::collections::hash_map::DefaultHasher::new();
                self.uri.hash(&mut hash);
                format!(
                    "/org/mpris/MediaPlayer2/track/t{id:x}_{:016x}",
                    hash.finish()
                )
            })
            .unwrap_or_else(|| "/org/mpris/MediaPlayer2/TrackList/NoTrack".into())
    }
    pub(super) fn status(&self) -> &str {
        if self.track.is_none() {
            "Stopped"
        } else if self.playing {
            "Playing"
        } else {
            "Paused"
        }
    }
    pub(super) fn metadata(&self) -> HashMap<String, OwnedValue> {
        let mut m = HashMap::new();
        if self.track.is_none() {
            return m;
        }
        m.insert(
            "mpris:trackid".into(),
            Value::from(OwnedObjectPath::try_from(self.track_path()).unwrap())
                .try_into()
                .unwrap(),
        );
        m.insert("xesam:title".into(), Str::from(self.title.clone()).into());
        m.insert("xesam:album".into(), Str::from(self.title.clone()).into());
        m.insert("xesam:url".into(), Str::from(self.uri.clone()).into());
        m.insert(
            "xesam:artist".into(),
            Value::from(vec![self.author.as_str()]).try_into().unwrap(),
        );
        if !self.art.is_empty() {
            m.insert("mpris:artUrl".into(), Str::from(self.art.clone()).into());
        }
        if let Some(duration) = self.duration {
            m.insert(
                "mpris:length".into(),
                ((duration.min(i64::MAX as u64 / 1000) * 1000) as i64).into(),
            );
        }
        m
    }
}
pub(super) struct Root {
    pub(super) tx: Sender<Command>,
}
#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        let _ = self.tx.send(Command::Show);
    }
    fn quit(&self) {
        let _ = self.tx.send(Command::Quit);
    }
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> &str {
        "Carlitos"
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        "io.github.mny315.Carlitos"
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec![]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        vec![]
    }
}
pub(super) struct Media {
    pub(super) tx: Sender<Command>,
    pub(super) state: Arc<RwLock<MediaState>>,
}
#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Media {
    fn next(&self) {
        let _ = self.tx.send(Command::Next(true));
    }
    fn previous(&self) {
        let _ = self.tx.send(Command::Next(false));
    }
    fn pause(&self) {
        let _ = self.tx.send(Command::Playing(false));
    }
    fn play(&self) {
        let _ = self.tx.send(Command::Playing(true));
    }
    fn play_pause(&self) {
        let _ = self.tx.send(Command::Toggle);
    }
    fn stop(&self) {
        let _ = self.tx.send(Command::Stop);
    }
    fn seek(&self, offset: i64) {
        let _ = self.tx.send(Command::SeekDelta(offset / 1000));
    }
    fn set_position(&self, track_id: OwnedObjectPath, position: i64) {
        let state = self.state.read().unwrap();
        if track_id.as_str() == state.track_path()
            && position >= 0
            && state.seekable
            && let Some(part) = state.track
        {
            let _ = self.tx.send(Command::SetPosition(
                part,
                state.uri.clone(),
                position as u64 / 1000,
            ));
        }
    }
    fn open_uri(&self, _uri: &str) -> zbus::fdo::Result<()> {
        Err(zbus::fdo::Error::NotSupported(
            "Import a folder in Carlitos".into(),
        ))
    }
    #[zbus(property)]
    fn playback_status(&self) -> String {
        self.state.read().unwrap().status().into()
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        self.state.read().unwrap().rate
    }
    #[zbus(property)]
    fn set_rate(&self, value: f64) -> zbus::fdo::Result<()> {
        if value.is_finite()
            && (carlitos::player::MIN_RATE..=carlitos::player::MAX_RATE).contains(&value)
        {
            let _ = self.tx.send(Command::Rate(value));
            Ok(())
        } else {
            Err(zbus::fdo::Error::InvalidArgs(
                "Speed must be between 0.5 and 3".into(),
            ))
        }
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        carlitos::player::MIN_RATE
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        carlitos::player::MAX_RATE
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        self.state.read().unwrap().metadata()
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.state.read().unwrap().volume
    }
    #[zbus(property)]
    fn set_volume(&self, value: f64) {
        if value.is_finite() {
            let _ = self.tx.send(Command::Volume(value));
        }
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        (self
            .state
            .read()
            .unwrap()
            .position
            .min(i64::MAX as u64 / 1000)
            * 1000) as i64
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.state.read().unwrap().next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.state.read().unwrap().previous
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.state.read().unwrap().track.is_some()
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.state.read().unwrap().track.is_some()
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.state.read().unwrap().seekable
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn an_old_desktop_track_cannot_seek_a_reused_database_id() {
        let (tx, rx) = mpsc::channel();
        let state = Arc::new(RwLock::new(MediaState {
            track: Some(1),
            uri: "file:///old-book/part.wav".into(),
            seekable: true,
            ..Default::default()
        }));
        let old = OwnedObjectPath::try_from(state.read().unwrap().track_path()).unwrap();
        state.write().unwrap().uri = "file:///new-book/part.wav".into();
        let media = Media { tx, state };
        media.set_position(old, 5_000_000);
        assert!(
            rx.try_recv().is_err(),
            "a removed track must not address its replacement"
        );
    }
}
