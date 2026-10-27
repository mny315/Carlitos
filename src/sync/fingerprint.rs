use crate::library::{Id, Library, Media};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    io::Read,
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Fingerprint {
    pub file: Id,
    pub signature: String,
    pub hash: String,
}
impl Fingerprint {
    pub fn signature(media: &Media) -> String {
        // Keep old/SAF cache signatures valid until precise local metadata is
        // available, then include it so same-second rescans invalidate hashes.
        if let Some(nanos) = media.modified_subsec_nanos {
            serde_json::to_string(&(
                &media.uri,
                &media.identity,
                media.size,
                media.modified,
                nanos,
            ))
            .unwrap()
        } else {
            serde_json::to_string(&(&media.uri, &media.identity, media.size, media.modified))
                .unwrap()
        }
    }
    pub fn matches(&self, media: &Media) -> bool {
        self.file == media.id && self.signature == Self::signature(media)
    }
    pub fn read(media: &Media, cancelled: &AtomicBool) -> Result<Self> {
        anyhow::ensure!(
            !cancelled.load(Ordering::Relaxed),
            "Fingerprinting cancelled"
        );
        #[cfg(target_os = "android")]
        if media.uri.starts_with("content:") {
            let before = crate::android::documents::stat(&media.uri)?;
            check_metadata(media, before.size, before.modified)?;
            let (hash, size) =
                hash_reader(&mut crate::android::documents::open(&media.uri)?, cancelled)?;
            let after = crate::android::documents::stat(&media.uri)?;
            check_metadata(media, after.size, after.modified)?;
            anyhow::ensure!(
                (before.size, before.modified) == (after.size, after.modified)
                    && before.size.is_none_or(|expected| expected == size),
                "Audio document changed while fingerprinting"
            );
            return Ok(Self {
                file: media.id,
                signature: Self::signature(media),
                hash,
            });
        }
        let path = crate::library::local_path(&media.uri).context("Not a local file")?;
        anyhow::ensure!(
            std::fs::symlink_metadata(&path)?.is_file(),
            "Not a regular audio file"
        );
        let mut reader = std::fs::File::open(&path)?;
        let before = reader.metadata()?;
        anyhow::ensure!(
            crate::platform::file_identity(&reader)? == media.identity,
            "Audio file was replaced; refresh its source"
        );
        check_metadata(
            media,
            Some(before.len()),
            Some(crate::platform::modified(&before)),
        )?;
        anyhow::ensure!(
            media
                .modified_subsec_nanos
                .is_none_or(|nanos| crate::platform::modified_subsec_nanos(&before) == Some(nanos)),
            "Audio file changed; refresh its source"
        );
        let (hash, size) = hash_reader(&mut reader, cancelled)?;
        let after = reader.metadata()?;
        // Use the open handle as well as the path: replacement by rename can
        // leave us reading an old inode, while in-place edits retain its ID.
        anyhow::ensure!(
            before.len() == after.len()
                && before.len() == size
                && before.modified().ok() == after.modified().ok()
                && crate::platform::identity(&path)? == media.identity,
            "Audio file changed while fingerprinting"
        );
        Ok(Self {
            file: media.id,
            signature: Self::signature(media),
            hash,
        })
    }
}

fn check_metadata(media: &Media, size: Option<u64>, modified: Option<i64>) -> Result<()> {
    anyhow::ensure!(
        media.size.is_none_or(|expected| size == Some(expected))
            && media
                .modified
                .is_none_or(|expected| modified == Some(expected)),
        "Audio file changed; refresh its source"
    );
    Ok(())
}

fn hash_reader(reader: &mut impl Read, cancelled: &AtomicBool) -> Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 256 * 1024];
    let mut size = 0;
    loop {
        anyhow::ensure!(
            !cancelled.load(Ordering::Relaxed),
            "Fingerprinting cancelled"
        );
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        size += count as u64;
        hasher.update(&buffer[..count]);
    }
    Ok((hex::encode(hasher.finalize()), size))
}

#[derive(Default)]
pub struct Index {
    pub books: BTreeMap<Id, BookKey>,
}
pub struct BookKey {
    pub key: String,
    pub parts: Vec<(Id, String, usize)>,
}
impl Index {
    pub(crate) fn books_by_key(&self) -> BTreeMap<&str, Vec<Id>> {
        let mut books = BTreeMap::<_, Vec<_>>::new();
        for (&id, book) in &self.books {
            books.entry(book.key.as_str()).or_default().push(id);
        }
        books
    }
    pub fn new(library: &Library, fingerprints: &[Fingerprint]) -> Self {
        let media: BTreeMap<_, _> = library.media.iter().map(|m| (m.id, m)).collect();
        let hashes: BTreeMap<_, _> = fingerprints
            .iter()
            .filter(|f| media.get(&f.file).is_some_and(|m| f.matches(m)))
            .map(|f| (f.file, &f.hash))
            .collect();
        // Build membership once: scanning every part again for every book
        // makes each published checkpoint quadratic in the library size.
        let mut parts_by_book = BTreeMap::<_, Vec<_>>::new();
        for part in &library.parts {
            parts_by_book.entry(part.book_id).or_default().push(part);
        }
        let mut books = BTreeMap::new();
        for book in &library.books {
            let mut parts = parts_by_book.remove(&book.id).unwrap_or_default();
            if parts.is_empty() || parts.iter().any(|p| !hashes.contains_key(&p.file_id)) {
                continue;
            }
            parts.sort_by_key(|part| part.ordinal);
            let mut occurrences = BTreeMap::new();
            let parts: Vec<_> = parts
                .into_iter()
                .map(|p| {
                    let hash = hashes[&p.file_id].clone();
                    let occurrence = occurrences.entry(hash.clone()).or_insert(0);
                    let entry = (p.id, hash, *occurrence);
                    *occurrence += 1;
                    entry
                })
                .collect();
            let mut sorted: Vec<_> = parts.iter().map(|(_, hash, _)| hash.as_str()).collect();
            sorted.sort();
            let mut hasher = Sha256::new();
            hasher.update(b"Carlitos book v1\0");
            for hash in sorted {
                hasher.update(hash.as_bytes());
            }
            books.insert(
                book.id,
                BookKey {
                    key: hex::encode(hasher.finalize()),
                    parts,
                },
            );
        }
        Self { books }
    }
}
