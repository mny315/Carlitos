#[cfg(target_os = "linux")]
use super::cache_cover;
use super::identity;
use crate::library::*;
#[cfg(target_os = "linux")]
use anyhow::bail;
use anyhow::{Context, Result};
#[cfg(target_os = "linux")]
use gst_pbutils::prelude::*;
use std::path::Path;

#[cfg(target_os = "linux")]
pub type Discoverer = gst_pbutils::Discoverer;
#[cfg(windows)]
pub use crate::windows::media::{Discoverer, discoverer, read_sort_tags};
#[cfg(target_os = "linux")]
pub fn discoverer() -> Result<Discoverer> {
    gst::init()?;
    Ok(gst_pbutils::Discoverer::new(gst::ClockTime::from_seconds(
        5,
    ))?)
}
#[cfg(windows)]
pub(super) fn read_media(path: &Path, root: &Path, discoverer: &Discoverer) -> Result<Media> {
    let metadata = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        metadata.is_file(),
        crate::i18n::tr("не обычный файл (ссылки не импортируются)")
    );
    let mut media = discoverer.read(path)?;
    media.uri = file_uri(path)?;
    media.relative = path
        .strip_prefix(root)?
        .to_str()
        .context(crate::i18n::tr("Путь не является UTF-8"))?
        .into();
    media.identity = identity(path)?;
    media.size = Some(metadata.len());
    media.modified = Some(crate::platform::modified(&metadata));
    Ok(media)
}

pub fn media_duration(uri: &str, discoverer: &Discoverer) -> Result<Option<u64>> {
    #[cfg(windows)]
    {
        discoverer.duration(uri)
    }
    #[cfg(target_os = "linux")]
    {
        let info = discoverer.discover_uri(uri)?;
        anyhow::ensure!(
            info.result() == gst_pbutils::DiscovererResult::Ok && !info.audio_streams().is_empty(),
            "Unsupported or damaged audio file"
        );
        Ok(info.duration().map(|d| d.mseconds()))
    }
}

#[cfg(target_os = "linux")]
pub(super) fn read_media(path: &Path, root: &Path, discoverer: &Discoverer) -> Result<Media> {
    let metadata = std::fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        bail!(crate::i18n::tr("не обычный файл (ссылки не импортируются)"));
    }
    let uri = file_uri(path)?;
    let info = discoverer.discover_uri(&uri)?;
    if info.result() != gst_pbutils::DiscovererResult::Ok || info.audio_streams().is_empty() {
        bail!(tformat!(
            "неподдерживаемый или повреждённый аудиофайл ({:?})",
            info.result()
        ));
    }
    let tags = discover_tags(&info);
    let sort_tags = sort_tags(&tags);
    macro_rules! tag {
        ($t:ty) => {
            tags.iter()
                .find_map(|t| t.get::<$t>().map(|v| v.get().to_owned()))
        };
    }
    let title = tag!(gst::tags::Title).unwrap_or_else(|| {
        path.file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    });
    let cover = tags
        .iter()
        .find_map(|t| {
            t.get::<gst::tags::Image>()
                .or_else(|| t.get::<gst::tags::PreviewImage>())
        })
        .and_then(|sample| {
            let sample = sample.get();
            let buffer = sample.buffer()?;
            let bytes = buffer.map_readable().ok()?;
            cache_cover(bytes.as_slice()).ok()
        });
    let mut chapters = vec![];
    if let Some(toc) = info.toc() {
        for entry in toc.entries() {
            collect_chapters(&entry, &mut chapters);
        }
    }
    chapters.sort_by_key(|c| c.start);
    let duration = info.duration().map(|d| d.mseconds());
    if chapters.is_empty()
        && let Ok(file) = std::fs::File::open(path)
    {
        chapters = super::chapters::read(&mut std::io::BufReader::new(file), duration);
    }
    Ok(Media {
        uri,
        relative: path
            .strip_prefix(root)?
            .to_str()
            .context(crate::i18n::tr("Путь не является UTF-8"))?
            .into(),
        identity: identity(path)?,
        size: Some(metadata.len()),
        modified: Some(crate::platform::modified(&metadata)),
        duration,
        title,
        artist: tag!(gst::tags::Artist).unwrap_or_default(),
        album: tag!(gst::tags::Album).unwrap_or_default(),
        year: sort_tags.year,
        genre: sort_tags.genre,
        sort_tags_read: true,
        track: tag!(gst::tags::TrackNumber),
        disc: tag!(gst::tags::AlbumVolumeNumber),
        cover,
        chapters,
        ..Default::default()
    })
}
#[cfg(target_os = "linux")]
fn discover_tags(info: &gst_pbutils::DiscovererInfo) -> Vec<gst::TagList> {
    let mut tags = vec![];
    // Since GStreamer 1.20, container-wide tags have their own accessor.
    // MP4 author/album/cover metadata need not appear on the audio stream.
    for container in info.container_streams() {
        if let Some(t) = container.tags() {
            tags.push(t);
        }
    }
    if let Some(t) = info.stream_info().and_then(|s| s.tags()) {
        tags.push(t);
    }
    for stream in info.audio_streams() {
        if let Some(t) = stream.tags() {
            tags.push(t);
        }
    }
    // Some demuxers expose global tags only on DiscovererInfo, even when
    // the stream/container accessors above are available.
    #[allow(deprecated)]
    if let Some(t) = info.tags() {
        tags.push(t);
    }
    tags
}

#[cfg(target_os = "linux")]
fn sort_tags(tags: &[gst::TagList]) -> BookTags {
    let year = tags.iter().find_map(|t| {
        t.get::<gst::tags::DateTime>()
            .filter(|v| v.get().has_year())
            .map(|v| v.get().year())
            .or_else(|| {
                t.get::<gst::tags::Date>()
                    .map(|v| i32::from(v.get().year()))
            })
    });
    let genre = tags
        .iter()
        .find_map(|t| {
            t.get::<gst::tags::Genre>()
                .map(|v| v.get().trim().to_owned())
        })
        .unwrap_or_default();
    BookTags { year, genre }
}

#[cfg(target_os = "linux")]
pub fn read_sort_tags(uri: &str, discoverer: &Discoverer) -> Result<BookTags> {
    let info = discoverer.discover_uri(uri)?;
    anyhow::ensure!(
        info.result() == gst_pbutils::DiscovererResult::Ok,
        "Could not read audio tags"
    );
    Ok(sort_tags(&discover_tags(&info)))
}

#[cfg(target_os = "linux")]
fn collect_chapters(entry: &gst::TocEntryRef, chapters: &mut Vec<Chapter>) {
    if entry.entry_type() == gst::TocEntryType::Chapter
        && let Some((start, end)) = entry.start_stop_times()
        && start >= 0
    {
        chapters.push(Chapter {
            title: entry
                .tags()
                .and_then(|t| t.get::<gst::tags::Title>().map(|s| s.get().to_string()))
                .unwrap_or_else(|| tformat!("Глава {}", chapters.len() + 1)),
            start: start as u64 / 1_000_000,
            end: (end >= start).then_some(end as u64 / 1_000_000),
        });
    }
    for child in entry.sub_entries() {
        collect_chapters(&child, chapters);
    }
}
