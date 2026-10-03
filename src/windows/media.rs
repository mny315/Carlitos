use super::{Apartment, decoder::Decoder};
use crate::library::{BookTags, Media, file_uri, local_path};
use anyhow::{Context, Result, bail};
use std::path::Path;

pub struct Discoverer {
    _apartment: Apartment,
}
pub fn discoverer() -> Result<Discoverer> {
    let apartment = Apartment::new()?;
    super::initialize_media()?;
    Ok(Discoverer {
        _apartment: apartment,
    })
}
impl Discoverer {
    pub fn duration(&self, uri: &str) -> Result<Option<u64>> {
        let mut decoder = Decoder::open(uri)?;
        // Negotiate and actually decode a packet so unsupported codecs fail at import.
        for _ in 0..32 {
            if !decoder.read()?.is_empty() {
                return Ok(decoder.duration);
            }
            if decoder.end {
                bail!("Empty or damaged audio file");
            }
        }
        bail!("Audio decoder produced no samples during inspection")
    }
    pub fn read(&self, path: &Path) -> Result<Media> {
        let duration = self.duration(&file_uri(path)?)?;
        let mut media = Media {
            duration,
            title: path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            sort_tags_read: true,
            ..Default::default()
        };
        let mut reader = std::io::BufReader::new(std::fs::File::open(path)?);
        crate::import::tags::read(&mut reader, &mut media);
        Ok(media)
    }
}
pub fn read_sort_tags(uri: &str, _discoverer: &Discoverer) -> Result<BookTags> {
    let path = local_path(uri).context("Expected local media")?;
    crate::import::tags::read_sort_tags(&mut std::io::BufReader::new(std::fs::File::open(path)?))
}
