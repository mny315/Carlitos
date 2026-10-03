mod chapters;
mod covers;
#[cfg(target_os = "android")]
mod documents;
mod filesystem;
mod grouping;
#[cfg_attr(target_os = "android", path = "../android/media.rs")]
mod media;
#[cfg(any(target_os = "android", test))]
mod parallel;
#[cfg(any(target_os = "android", test))]
pub(crate) mod reader;
#[cfg(any(windows, target_os = "android", test))]
pub(crate) mod tags;

pub(crate) use covers::cache_cover;
use covers::read_cover;
pub use covers::{custom_cover, custom_cover_location, load_cover_image};
pub use filesystem::scan;
use grouping::{book_metadata, book_root};
use media::read_media;
pub use media::{Discoverer, discoverer, media_duration, read_sort_tags};

use crate::{library::*, storage::Draft};
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ImportMode {
    Book,
    Books,
}
#[derive(Default, Clone)]
pub struct ScanControl {
    pub cancelled: Arc<AtomicBool>,
}
impl ScanControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
    pub(crate) fn check(&self) -> Result<()> {
        if self.cancelled.load(Ordering::Relaxed) {
            bail!(crate::i18n::tr("Импорт отменён"));
        }
        Ok(())
    }
}
pub struct ScanResult {
    pub drafts: Vec<Draft>,
    pub issues: Vec<String>,
}
pub fn identity(path: &Path) -> Result<String> {
    crate::platform::identity(path)
}
pub(crate) fn is_audio(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["mp3", "flac", "wav", "wave", "m4a", "m4b", "aac"]
            .contains(&e.to_ascii_lowercase().as_str())
    })
}

pub fn scan_location(
    location: crate::source::Location,
    mode: ImportMode,
    control: ScanControl,
) -> Result<ScanResult> {
    match location {
        crate::source::Location::File(path) => scan(vec![path], mode, control),
        #[cfg(target_os = "android")]
        crate::source::Location::Document(uri) => documents::scan(&uri, mode, control),
        #[cfg(not(target_os = "android"))]
        crate::source::Location::Document(_) => bail!("Document sources require Android"),
    }
}
