use crate::library::{Chapter, Millis};
use std::io::{Read, Seek};

/// Read both Nero chapter lists and QuickTime chapter tracks. GStreamer's
/// discoverer does not always expose them as a TOC. Artwork and other tags
/// were already read by the platform metadata reader, so skip them here.
pub(super) fn read(reader: &mut (impl Read + Seek), duration: Option<Millis>) -> Vec<Chapter> {
    if reader.rewind().is_err() {
        return vec![];
    }
    let mut header = [0; 12];
    if reader.read_exact(&mut header).is_err() || &header[4..8] != b"ftyp" {
        return vec![];
    }
    // Match mp4ameta's preference for the Nero list, but read the formats
    // independently: damage in one must not hide a usable copy in the other.
    for read_chapter_list in [true, false] {
        if reader.rewind().is_err() {
            return vec![];
        }
        let config = mp4ameta::ReadConfig {
            read_chapter_list,
            read_chapter_track: !read_chapter_list,
            ..mp4ameta::ReadConfig::NONE
        };
        // mp4ameta 0.13 can panic on malformed movie/chapter timescales instead
        // of returning an error. Chapters are optional; keep the scan worker
        // alive so it can report completion and continue with the next file.
        let Ok(Ok(tag)) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            mp4ameta::Tag::read_with(reader, &config)
        })) else {
            continue;
        };
        let chapters = tag.chapters();
        if chapters.is_empty() {
            continue;
        }
        return chapters
            .iter()
            .enumerate()
            .map(|(index, chapter)| Chapter {
                title: if chapter.title.is_empty() {
                    tformat!("Глава {}", index + 1)
                } else {
                    chapter.title.clone()
                },
                start: chapter.start.as_millis().min(u64::MAX as u128) as u64,
                end: chapters
                    .get(index + 1)
                    .map(|next| next.start.as_millis().min(u64::MAX as u128) as u64)
                    .or(duration),
            })
            .collect();
    }
    vec![]
}

#[cfg(test)]
mod tests;
