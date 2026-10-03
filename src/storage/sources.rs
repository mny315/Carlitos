use super::{Store, load_session, save_session};
use crate::library::*;
use anyhow::{Context, Result, bail};
use rusqlite::params;
use std::path::Path;

impl Store {
    pub fn relocate_location(
        &mut self,
        source: Id,
        root: &crate::source::Location,
        control: &crate::import::ScanControl,
    ) -> Result<Library> {
        match root {
            crate::source::Location::File(path) => self.relocate_cancellable(source, path, control),
            #[cfg(target_os = "android")]
            crate::source::Location::Document(uri) => self.relocate_document(source, uri, control),
            #[cfg(not(target_os = "android"))]
            crate::source::Location::Document(_) => bail!("Document sources require Android"),
        }
    }
    pub fn remove_source(&mut self, id: Id) -> Result<Library> {
        let tx = self.conn.transaction()?;
        let document_links: Vec<Id> = tx.prepare("SELECT m.id FROM media_files m JOIN source_files sf ON sf.file_id=m.id WHERE sf.source_id=?1 AND m.uri LIKE 'content:%'")?
            .query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?;
        tx.execute("DELETE FROM books WHERE source_id=?1", [id])?;
        tx.execute("DELETE FROM source_files WHERE source_id=?1", [id])?;
        // Overlapping imports can add this source's files to another book.
        // Remove only parts with no remaining source; shared files keep their IDs.
        tx.execute("DELETE FROM book_parts WHERE file_id IN (SELECT id FROM media_files WHERE source_id=?1 AND NOT EXISTS(SELECT 1 FROM source_files WHERE file_id=media_files.id))", [id])?;
        tx.execute("DELETE FROM media_files WHERE NOT EXISTS(SELECT 1 FROM source_files WHERE file_id=media_files.id) AND NOT EXISTS(SELECT 1 FROM book_parts WHERE file_id=media_files.id)",[])?;
        let mut remaining: Vec<(Id, Id, String, String)> = tx.prepare(
            "SELECT m.id,s.id,s.uri,sf.relative FROM media_files m JOIN source_files sf ON sf.file_id=m.id JOIN sources s ON s.id=sf.source_id WHERE m.source_id=?1 AND sf.source_id=(SELECT MIN(source_id) FROM source_files WHERE file_id=m.id)",
        )?.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?;
        // A recent overlapping import may have renewed the URI through the
        // removed grant without changing the original owner of the recording.
        for file in document_links {
            if remaining.iter().any(|(id, _, _, _)| *id == file) {
                continue;
            }
            use rusqlite::OptionalExtension;
            if let Some(link) = tx.query_row(
                "SELECT m.id,s.id,s.uri,sf.relative FROM media_files m JOIN source_files sf ON sf.file_id=m.id JOIN sources s ON s.id=sf.source_id WHERE m.id=?1 ORDER BY sf.source_id=m.source_id DESC,sf.source_id LIMIT 1",
                [file], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            ).optional()? { remaining.push(link); }
        }
        for (file, source, root, relative) in remaining {
            let uri = if let Some(root) = local_path(&root) {
                file_uri(&root.join(&relative))?
            } else {
                let uri: String =
                    tx.query_row("SELECT uri FROM media_files WHERE id=?1", [file], |r| {
                        r.get(0)
                    })?;
                crate::source::document_uri_in_source(&uri, &root)?
            };
            // Shared files must now resolve through a surviving source, even
            // when the removed source was an independent hardlink alias.
            tx.execute(
                "UPDATE media_files SET source_id=?1,relative=?2,uri=?3 WHERE id=?4",
                params![source, relative, uri, file],
            )?;
        }
        tx.execute("DELETE FROM sources WHERE id=?1", [id])?;
        save_session(&tx, &load_session(&tx)?)?;
        tx.commit()?;
        self.load()
    }
    /// Relocation is all-or-nothing and verifies relative paths, sizes and decoded durations.
    pub fn relocate(&mut self, source: Id, new_root: &Path) -> Result<Library> {
        self.relocate_cancellable(source, new_root, &crate::import::ScanControl::default())
    }
    pub fn relocate_cancellable(
        &mut self,
        source: Id,
        new_root: &Path,
        control: &crate::import::ScanControl,
    ) -> Result<Library> {
        control.check()?;
        let new_root = dunce::canonicalize(new_root)
            .with_context(|| tformat!("Не найден {}", new_root.display()))?;
        anyhow::ensure!(
            new_root.is_dir(),
            crate::i18n::tr("Укажите путь к существующей папке")
        );
        let lib = self.load()?;
        let src = lib
            .sources
            .iter()
            .find(|s| s.id == source)
            .context(crate::i18n::tr("Источник не найден"))?;
        let old_root = local_path(&src.uri)
            .context(crate::i18n::tr("Источник не является локальной папкой"))?;
        let discoverer = crate::import::discoverer()?;
        let mut updates = std::collections::BTreeMap::new();
        let mut sources = vec![];
        let mut links = std::collections::BTreeSet::new();
        // A parent folder move also moves independently imported child folders.
        // Include every child's files in validation before changing any paths.
        for source in &lib.sources {
            let Some(root) = local_path(&source.uri) else {
                continue;
            };
            let Ok(relative_root) = root.strip_prefix(&old_root) else {
                continue;
            };
            sources.push((source.id, file_uri(&new_root.join(relative_root))?));
            let mut statement = self
                .conn
                .prepare("SELECT file_id,relative FROM source_files WHERE source_id=?1")?;
            for link in statement.query_map([source.id], |r| {
                Ok((r.get::<_, Id>(0)?, r.get::<_, String>(1)?))
            })? {
                let (file, relative) = link?;
                // Check before joining: an absolute child would discard its prefix.
                let relative = Path::new(&relative);
                if relative.is_absolute() {
                    bail!(crate::i18n::tr("Недопустимый относительный путь"));
                }
                links.insert((file, relative_root.join(relative)));
            }
        }
        for (file_id, relative) in links {
            control.check()?;
            let file = lib
                .media(file_id)
                .context(crate::i18n::tr("Файл не найден в библиотеке"))?;
            let relative = relative.as_path();
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|c| matches!(c, std::path::Component::ParentDir))
            {
                bail!(crate::i18n::tr("Недопустимый относительный путь"));
            }
            let path = new_root.join(relative);
            let metadata = std::fs::symlink_metadata(&path)
                .with_context(|| tformat!("Не найден {}", path.display()))?;
            anyhow::ensure!(
                metadata.is_file(),
                crate::i18n::tr("не обычный файл (ссылки не импортируются)")
            );
            if Some(metadata.len()) != file.size {
                bail!(tformat!(
                    "Размер файла отличается: {}. Перепривязка отменена.",
                    relative.display()
                ));
            }
            let uri = file_uri(&path)?;
            let duration = crate::import::media_duration(&uri, &discoverer)?;
            if file.duration.is_some() && duration != file.duration {
                bail!(tformat!("Длительность отличается: {}", relative.display()));
            }
            let update = (
                uri,
                crate::import::identity(&path)?,
                crate::platform::modified(&metadata),
            );
            let canonical = local_path(&file.uri).is_some_and(|old| old == old_root.join(relative));
            if canonical {
                updates.insert(file.id, update);
            } else {
                updates.entry(file.id).or_insert(update);
            }
        }
        control.check()?;
        let tx = self.conn.transaction()?;
        let moved_sources: std::collections::HashSet<_> =
            sources.iter().map(|(id, _)| *id).collect();
        for (id, uri) in sources {
            tx.execute("UPDATE sources SET uri=?1 WHERE id=?2", params![uri, id])?;
        }
        for (id, (uri, identity, modified)) in updates {
            // Links from an unchanged parent otherwise keep pointing at the
            // old child folder and make the next parent relocation fail.
            let links: Vec<(Id, String, String)> = tx.prepare(
                "SELECT s.id,s.uri,sf.relative FROM source_files sf JOIN sources s ON s.id=sf.source_id WHERE sf.file_id=?1",
            )?.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            for (source, uri, relative) in links {
                if moved_sources.contains(&source) {
                    continue;
                }
                let Some(root) = local_path(&uri) else {
                    continue;
                };
                let old_path = root.join(relative);
                let Ok(suffix) = old_path.strip_prefix(&old_root) else {
                    // Independent hardlink aliases outside the moved tree
                    // still belong to their original source.
                    continue;
                };
                let path = new_root.join(suffix);
                if let Ok(relative) = path.strip_prefix(&root) {
                    tx.execute(
                        "UPDATE source_files SET relative=?1 WHERE source_id=?2 AND file_id=?3",
                        params![relative.to_string_lossy(), source, id],
                    )?;
                } else {
                    tx.execute(
                        "DELETE FROM source_files WHERE source_id=?1 AND file_id=?2",
                        params![source, id],
                    )?;
                }
            }
            // A moved file can have been first imported through that parent.
            // Keep its owning source and relative path consistent with its URI.
            let links: Vec<(Id, String, String)> = tx.prepare(
                "SELECT s.id,s.uri,sf.relative FROM source_files sf JOIN sources s ON s.id=sf.source_id WHERE sf.file_id=?1 ORDER BY sf.source_id=(SELECT source_id FROM media_files WHERE id=?1) DESC,sf.source_id",
            )?.query_map([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            let path = local_path(&uri);
            let (owner, _, relative) = links
                .into_iter()
                .find(|(_, root, relative)| {
                    local_path(root).map(|root| root.join(relative)) == path
                })
                .context("Relocated file has no matching source path")?;
            tx.execute(
                "UPDATE media_files SET uri=?1,identity=?2,modified=?3,source_id=?5,relative=?6 WHERE id=?4",
                params![uri, identity, modified, id, owner, relative],
            )?;
        }
        tx.commit()?;
        self.load()
    }
}
