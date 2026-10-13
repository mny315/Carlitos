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
        serde_json::to_string(&(&media.uri, &media.identity, media.size, media.modified)).unwrap()
    }
    pub fn matches(&self, media: &Media) -> bool {
        self.file == media.id && self.signature == Self::signature(media)
    }
    pub fn read(media: &Media, cancelled: &AtomicBool) -> Result<Self> {
        #[cfg(target_os = "android")]
        let mut reader: Box<dyn Read> = if media.uri.starts_with("content:") {
            Box::new(crate::android::documents::open(&media.uri)?)
        } else {
            Box::new(std::fs::File::open(
                crate::library::local_path(&media.uri).context("Not a local file")?,
            )?)
        };
        #[cfg(not(target_os = "android"))]
        let mut reader = {
            let path = crate::library::local_path(&media.uri).context("Not a local file")?;
            anyhow::ensure!(
                std::fs::metadata(&path)?.is_file(),
                "Not a regular audio file"
            );
            std::fs::File::open(path)?
        };
        let mut hasher = Sha256::new();
        let mut buffer = vec![0; 256 * 1024];
        loop {
            anyhow::ensure!(
                !cancelled.load(Ordering::Relaxed),
                "Fingerprinting cancelled"
            );
            let count = reader.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok(Self {
            file: media.id,
            signature: Self::signature(media),
            hash: hex::encode(hasher.finalize()),
        })
    }
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
    pub fn new(library: &Library, fingerprints: &[Fingerprint]) -> Self {
        let media: BTreeMap<_, _> = library.media.iter().map(|m| (m.id, m)).collect();
        let hashes: BTreeMap<_, _> = fingerprints
            .iter()
            .filter(|f| media.get(&f.file).is_some_and(|m| f.matches(m)))
            .map(|f| (f.file, &f.hash))
            .collect();
        let mut books = BTreeMap::new();
        for book in &library.books {
            let parts = library.book_parts(book.id);
            if parts.is_empty() || parts.iter().any(|p| !hashes.contains_key(&p.file_id)) {
                continue;
            }
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
