//! Android validates audio; the portable reader supplies tags and chapters.
use crate::library::{BookTags, Media};
use anyhow::{Result, bail};
use std::path::Path;

pub struct Discoverer;
pub fn discoverer() -> Result<Discoverer> {
    Ok(Discoverer)
}
pub fn media_duration(uri: &str, _discoverer: &Discoverer) -> Result<Option<u64>> {
    Ok(crate::android::documents::probe(uri)?.duration)
}
pub fn read_sort_tags(uri: &str, _discoverer: &Discoverer) -> Result<BookTags> {
    crate::import::tags::read_sort_tags(&mut std::io::BufReader::new(
        crate::android::documents::open(uri)?,
    ))
}
pub(super) fn read_media(_path: &Path, _root: &Path, _discoverer: &Discoverer) -> Result<Media> {
    bail!("Choose an Android document or folder through the system picker")
}
