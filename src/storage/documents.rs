use super::Store;
use crate::{android::documents, import::ScanControl, library::*, source::document_identity};
use anyhow::{Context, Result};
use rusqlite::params;
use std::collections::{BTreeMap, HashMap, HashSet};

impl Store {
    pub(super) fn relocate_document(
        &mut self,
        source: Id,
        uri: &str,
        control: &ScanControl,
    ) -> Result<Library> {
        control.check()?;
        let root = documents::stat(uri)?;
        anyhow::ensure!(root.directory, "Choose the source folder");
        let library = self.load()?;
        let original = library
            .sources
            .iter()
            .find(|s| s.id == source)
            .context("Source not found")?;
        let original_identity =
            document_identity(&original.uri).context("Source is not an Android document")?;
        let same_root = original_identity == root.identity()?;
        let all_links: Vec<(Id, Id, String)> = self
            .conn
            .prepare("SELECT source_id,file_id,relative FROM source_files")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        let parent_links: HashMap<_, _> = all_links
            .iter()
            .filter(|(s, _, _)| *s == source)
            .map(|(_, file, relative)| (*file, relative.as_str()))
            .collect();
        let mut sources = BTreeMap::from([(source, root.uri.clone())]);
        let mut file_sources = HashSet::new();
        // Stored relative names establish nested sources even when the old tree
        // is gone. Never split provider document IDs to guess a filesystem path.
        for candidate in &library.sources {
            if candidate.id == source || document_identity(&candidate.uri).is_none() {
                continue;
            }
            let shared: Vec<_> = all_links
                .iter()
                .filter(|(s, file, _)| *s == candidate.id && parent_links.contains_key(file))
                .collect();
            if shared.is_empty() {
                continue;
            }
            // A separately opened file is itself a source, not its parent folder.
            if shared.len() == 1 {
                let (_, file, _) = shared[0];
                if library
                    .media(*file)
                    .is_some_and(|m| document_identity(&candidate.uri) == document_identity(&m.uri))
                {
                    let moved = documents::relative(&root.uri, parent_links[file])?;
                    anyhow::ensure!(!moved.directory, "Expected the nested source recording");
                    sources.insert(candidate.id, moved.uri);
                    file_sources.insert(candidate.id);
                    continue;
                }
            }
            let prefixes: Option<Vec<_>> = shared
                .iter()
                .map(|(_, file, relative)| {
                    let parent = parent_links[file];
                    if parent == relative {
                        Some("")
                    } else {
                        parent.strip_suffix(relative.as_str())?.strip_suffix('/')
                    }
                })
                .collect();
            let Some(prefixes) = prefixes else {
                continue;
            };
            if prefixes[0].is_empty() || prefixes.iter().any(|p| p != &prefixes[0]) {
                continue;
            }
            let moved = documents::relative(&root.uri, prefixes[0])?;
            anyhow::ensure!(moved.directory, "Nested source is not a folder");
            sources.insert(candidate.id, moved.uri);
        }
        let mut updates = BTreeMap::<Id, Media>::new();
        for (owner, file, relative) in &all_links {
            let Some(source_uri) = sources.get(owner) else {
                continue;
            };
            control.check()?;
            let previous = library.media(*file).context("Recording not found")?;
            let candidate = if file_sources.contains(owner) {
                documents::stat(source_uri)?
            } else {
                documents::relative(source_uri, relative)?
            };
            let candidate = documents::probe(&candidate.uri)?;
            let identity = candidate.identity()?;
            let same_document = document_identity(&previous.uri).as_ref() == Some(&identity);
            anyhow::ensure!(
                same_document
                    || (!same_root
                        && previous.size.is_some()
                        && candidate.size == previous.size
                        && previous.duration.is_some()
                        && candidate.duration == previous.duration),
                "Cannot verify {relative}; the document differs or size/duration is unknown. Source unchanged."
            );
            if let Some(existing) = updates.get(file) {
                anyhow::ensure!(
                    existing.identity == identity,
                    "Ambiguous document mapping: {relative}. Source unchanged."
                );
                if *owner != previous.source_id {
                    continue;
                }
            }
            let mut media = previous.clone();
            media.uri = candidate.uri;
            media.identity = identity;
            media.size = candidate.size;
            media.modified = candidate.modified;
            media.source_id = *owner;
            media.relative = relative.clone();
            updates.insert(*file, media);
        }
        // Two old files cannot silently collapse onto a single new document.
        let mut identities = HashSet::new();
        for media in updates.values() {
            anyhow::ensure!(
                identities.insert(&media.identity),
                "Ambiguous destination documents. Source unchanged."
            );
            anyhow::ensure!(
                !library
                    .media
                    .iter()
                    .any(|m| m.id != media.id && m.identity == media.identity),
                "Destination already belongs to another recording. Source unchanged."
            );
        }
        control.check()?;
        let tx = self.conn.transaction()?;
        for (id, uri) in &sources {
            tx.execute("UPDATE sources SET uri=?1 WHERE id=?2", params![uri, id])?;
        }
        for file in updates.values() {
            let previous = library.media(file.id).context("Recording not found")?;
            // A changed document is no longer reachable through an unmoved
            // overlapping source. Its parts/progress still refer to this file ID.
            if document_identity(&previous.uri) != document_identity(&file.uri) {
                for (source, linked, _) in &all_links {
                    if *linked == file.id && !sources.contains_key(source) {
                        tx.execute(
                            "DELETE FROM source_files WHERE source_id=?1 AND file_id=?2",
                            params![source, file.id],
                        )?;
                    }
                }
            }
            tx.execute(
                "UPDATE media_files SET uri=?1,identity=?2,size=?3,modified=?4,data=?5,source_id=?7,relative=?8 WHERE id=?6",
                params![file.uri, file.identity, super::stored_size(file.size)?, file.modified.unwrap_or(-1),
                    serde_json::to_string(file)?, file.id, file.source_id, file.relative],
            )?;
        }
        tx.commit()?;
        self.load()
    }
}
