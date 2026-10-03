use super::audio::Playback;
use crate::source::Location;
use crate::{
    import::ScanResult,
    library::{BookTags, Id, Library, Media},
    settings::Settings,
    storage::Draft,
};
use anyhow::Result;

pub enum Command {
    #[cfg(all(target_os = "android", feature = "android-playback-tests"))]
    PlaybackTest(
        serde_json::Value,
        std::sync::mpsc::Sender<anyhow::Result<serde_json::Value>>,
    ),
    #[cfg(target_os = "android")]
    OpenDocument(Media),
    #[cfg(target_os = "android")]
    Picked(String, String, String, String),
    Resume(Id),
    Part(Id, u64),
    Toggle,
    Playing(bool),
    Stop,
    Next(bool),
    SeekDelta(i64),
    SeekAbsolute(u64),
    SetPosition(Id, String, u64),
    SeekFraction(f64),
    Exact(String),
    Volume(f64),
    Rate(f64),
    RateStep(bool),
    SkipSilence(bool),
    Mute,
    Complete(Id),
    ResetProgress(Id),
    MovePart(Id, bool),
    Edit(Id, String, String, Vec<Id>, Option<String>),
    PrepareCover(u64, Location),
    TagsRead(Vec<(Media, BookTags)>),
    RemoveBook(Id),
    RemoveSource(Id),
    Relocate(Id, Location),
    Rescan(Id),
    Scan(Location, bool),
    CancelScan,
    ScanDone(u64, Result<ScanResult>),
    DraftEdit(usize, String, String, bool),
    DraftFile(usize, String, String),
    Import,
    Settings(Settings),
    Playback(Playback),
    AudioFailed(u64, String),
    AudioStopped,
    Relocated(Id, Result<()>),
    Error(String),
    Hidden(bool),
    Show,
    Quit,
}
pub enum Event {
    #[cfg(target_os = "android")]
    AndroidRefresh,
    #[cfg(target_os = "android")]
    Picked(String, String, String, String),
    Library(Box<Library>),
    Playback(Playback),
    Drafts(Vec<Draft>, Vec<String>),
    Volume(f64, bool),
    Scanning(bool, String),
    Settings(Settings),
    Notice(String),
    Imported,
    BookEdited(Id),
    CoverPrepared(u64, Result<String>),
    SourceUpdating(bool),
    SourceUpdated(Id, String),
    Show,
    Hidden(bool),
    Quit,
}
