//! Portable metadata only. Platform decoders remain responsible for duration
//! and checking that audio actually decodes.
use crate::library::{BookTags, Media};
use anyhow::Result;
use lofty::{config::ParseOptions, prelude::*, probe::Probe};
use std::io::{Read, Seek};

fn tagged(reader: &mut (impl Read + Seek)) -> Result<lofty::file::TaggedFile> {
    reader.rewind()?;
    Ok(Probe::new(reader)
        .options(ParseOptions::new().read_properties(false))
        .guess_file_type()?
        .read()?)
}

pub(crate) fn read_sort_tags(reader: &mut (impl Read + Seek)) -> Result<BookTags> {
    let tagged = tagged(reader)?;
    let tag = tagged.primary_tag().or_else(|| tagged.first_tag());
    Ok(BookTags {
        year: tag.and_then(|t| t.date()).map(|v| i32::from(v.year)),
        genre: tag
            .and_then(|t| t.genre())
            .map(|g| g.into_owned())
            .unwrap_or_default(),
    })
}

pub(crate) fn read(reader: &mut (impl Read + Seek), media: &mut Media) {
    read_with_cover(reader, media, super::cache_cover);
}

fn read_with_cover(
    reader: &mut (impl Read + Seek),
    media: &mut Media,
    save_cover: impl FnOnce(&[u8]) -> Result<String>,
) {
    if let Ok(tagged) = tagged(reader) {
        media.sort_tags_read = true;
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            media.title = tag
                .title()
                .map(|v| v.into_owned())
                .unwrap_or_else(|| media.title.clone());
            media.artist = tag.artist().map(|v| v.into_owned()).unwrap_or_default();
            media.album = tag.album().map(|v| v.into_owned()).unwrap_or_default();
            media.year = tag.date().map(|v| i32::from(v.year));
            media.genre = tag.genre().map(|v| v.into_owned()).unwrap_or_default();
            media.track = tag.track();
            media.disc = tag.disk();
            if let Some(picture) = tag.pictures().first() {
                media.cover = save_cover(picture.data()).ok();
            }
        }
    }
    media.chapters = super::chapters::read(reader, media.duration);
}

#[cfg(test)]
mod tests;
