use crate::library::cache_dir;
use anyhow::{Context, Result, bail};
use std::{
    cell::RefCell,
    collections::HashMap,
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    time::SystemTime,
};

type CoverStamp = (u64, SystemTime);
thread_local! {
    // Scan workers reuse a validated thumbnail without decoding its PNG again
    // for every chapter. A changed/deleted file always invalidates the entry.
    static VERIFIED: RefCell<HashMap<PathBuf, CoverStamp>> = RefCell::new(HashMap::new());
}

fn cover_stamp(path: &Path) -> Option<CoverStamp> {
    let metadata = std::fs::metadata(path).ok()?;
    metadata
        .is_file()
        .then_some((metadata.len(), metadata.modified().ok()?))
}

fn remember_cover(path: &Path, stamp: CoverStamp) {
    VERIFIED.with_borrow_mut(|verified| {
        if verified.len() >= 128 {
            verified.clear();
        }
        verified.insert(path.to_owned(), stamp);
    });
}

fn cached_cover_is_valid(path: &Path) -> bool {
    let Some(stamp) = cover_stamp(path) else {
        return false;
    };
    if VERIFIED.with_borrow(|verified| verified.get(path) == Some(&stamp)) {
        return true;
    }
    let valid = load_cover_image(path).is_ok_and(|img| {
        img.width() > 0 && img.height() > 0 && img.width() <= 256 && img.height() <= 256
    });
    if valid {
        remember_cover(path, stamp);
    }
    valid
}

pub(crate) fn cache_cover(bytes: &[u8]) -> Result<String> {
    // Chapters commonly carry identical artwork. Its content, rather than the
    // recording URI, identifies the thumbnail across files and source moves.
    save_cover(bytes, "embedded", &cache_dir().join("covers"))
}

pub fn custom_cover(path: &Path, directory: &Path) -> Result<String> {
    save_cover(&read_cover(path)?, "custom", directory)
}

pub(super) fn read_cover(path: &Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let metadata = std::fs::metadata(path)?;
    anyhow::ensure!(
        metadata.is_file(),
        crate::i18n::tr("Обложка должна быть обычным файлом")
    );
    if metadata.len() > 10 * 1024 * 1024 {
        bail!(crate::i18n::tr("Обложка больше 10 МиБ"));
    }
    // Also bound the read if the file grows after the metadata check.
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(10 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub fn load_cover_image(path: &Path) -> Result<image::DynamicImage> {
    decode_cover(&read_cover(path)?)
}

fn decode_cover(bytes: &[u8]) -> Result<image::DynamicImage> {
    use image::ImageDecoder;
    if bytes.len() > 10 * 1024 * 1024 {
        bail!(crate::i18n::tr("Обложка больше 10 МиБ"));
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits.clone());
    let mut decoder = reader.into_decoder()?;
    // ImageReader::decode reserves the output buffer as well as decoder memory.
    // Retain that bound when reading orientation through the decoder directly.
    limits.reserve(decoder.total_bytes())?;
    decoder.set_limits(limits)?;
    let orientation = decoder.orientation()?;
    let mut image = image::DynamicImage::from_decoder(decoder)?;
    image.apply_orientation(orientation);
    Ok(image)
}

pub(super) fn save_cover(bytes: &[u8], key: &str, dir: &Path) -> Result<String> {
    if bytes.len() > 10 * 1024 * 1024 {
        bail!(crate::i18n::tr("Обложка больше 10 МиБ"));
    }
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    // Older thumbnails ignored Exif orientation; do not reuse those pixels.
    "exif-oriented-v1".hash(&mut hash);
    key.hash(&mut hash);
    bytes.hash(&mut hash);
    let path = dir.join(format!("{:016x}.png", hash.finish()));
    // Validate the small cached PNG before returning it, so interrupted or
    // damaged cache files are still repaired without decoding the large source
    // image and encoding the same thumbnail for every chapter or rescan.
    if cached_cover_is_valid(&path) {
        return cover_path(&path);
    }
    let img = decode_cover(bytes)?;
    let img = img.thumbnail(img.width().min(256), img.height().min(256));
    std::fs::create_dir_all(dir)?;
    let mut encoded = std::io::Cursor::new(Vec::new());
    img.write_to(&mut encoded, image::ImageFormat::Png)?;
    let encoded = encoded.into_inner();
    if !read_cover(&path).is_ok_and(|cached| cached == encoded) {
        use std::io::Write;
        // Readers see either the complete old image or the complete new one.
        // An interrupted earlier write is repaired rather than cached forever.
        let mut temp = tempfile::NamedTempFile::new_in(dir)?;
        temp.write_all(&encoded)?;
        temp.as_file().sync_all()?;
        temp.persist(&path)?;
    }
    if let Some(stamp) = cover_stamp(&path) {
        remember_cover(&path, stamp);
    }
    cover_path(&path)
}

fn cover_path(path: &Path) -> Result<String> {
    Ok(dunce::canonicalize(path)?
        .to_str()
        .context(crate::i18n::tr("Путь не является UTF-8"))?
        .to_owned())
}

pub fn custom_cover_location(
    location: &crate::source::Location,
    directory: &Path,
) -> Result<String> {
    match location {
        crate::source::Location::File(path) => custom_cover(path, directory),
        #[cfg(target_os = "android")]
        crate::source::Location::Document(uri) => save_cover(
            &crate::android::documents::read_cover(uri)?,
            "custom",
            directory,
        ),
        #[cfg(not(target_os = "android"))]
        crate::source::Location::Document(_) => bail!("Document sources require Android"),
    }
}

#[cfg(test)]
mod tests;
