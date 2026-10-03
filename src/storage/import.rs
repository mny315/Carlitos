mod documents;
mod files;
mod grouping;

use super::{Draft, Store};
use crate::library::*;
use anyhow::{Result, bail};
use files::existing_file;
use grouping::split_combined_books;
use rusqlite::{OptionalExtension, params};

impl Store {
    /// An entire confirmed preview is committed together. Existing parts retain IDs, order and progress.
    pub fn import(&mut self, drafts: Vec<Draft>) -> Result<Library> {
        documents::validate_paths(&drafts)?;
        let tx = self.conn.transaction()?;
        split_combined_books(&tx, &drafts)?;
        let mut refresh_covers = std::collections::BTreeSet::new();
        for draft in drafts
            .into_iter()
            .filter(|d| d.include && !d.files.is_empty())
        {
            // Re-selecting the same folder through another tree renews its URI,
            // retaining the original source/book IDs.
            if let Some(identity) = crate::source::document_identity(&draft.root_uri) {
                let sources: Vec<(Id, String)> = tx
                    .prepare("SELECT id,uri FROM sources WHERE kind='book'")?
                    .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?;
                if let Some((id, _)) = sources.iter().find(|(_, uri)| {
                    crate::source::document_identity(uri).as_ref() == Some(&identity)
                }) {
                    tx.execute(
                        "UPDATE sources SET uri=?1 WHERE id=?2",
                        params![draft.root_uri, id],
                    )?;
                }
            }
            let kind = "book";
            tx.execute(
                "INSERT INTO sources(uri,kind) VALUES(?1,?2) ON CONFLICT(uri,kind) DO NOTHING",
                params![draft.root_uri, kind],
            )?;
            let source: Id = tx.query_row(
                "SELECT id FROM sources WHERE uri=?1 AND kind=?2",
                params![draft.root_uri, kind],
                |r| r.get(0),
            )?;
            let mut file_ids = Vec::with_capacity(draft.files.len());
            let mut seen_files = std::collections::HashSet::with_capacity(draft.files.len());
            for file in &draft.files {
                let existing = existing_file(&tx, file)?;
                let id = if let Some(id) = existing {
                    let (data, size, modified, identity): (String, i64, i64, String) = tx
                        .query_row(
                            "SELECT data,size,modified,identity FROM media_files WHERE id=?1",
                            [id],
                            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                        )?;
                    let mut refreshed = file.clone();
                    let previous: Media = serde_json::from_str(&data)?;
                    // Discoverer may omit a TOC on a later scan of the same file.
                    // Keep known chapters only while the recording is unchanged.
                    if refreshed.chapters.is_empty()
                        && file.size.is_some()
                        && file.modified.is_some()
                        && (size, modified, identity.as_str())
                            == (
                                super::stored_size(file.size)?,
                                file.modified.unwrap_or(-1),
                                file.identity.as_str(),
                            )
                    {
                        refreshed.chapters = previous.chapters;
                    }
                    // Refresh tag-derived titles, retaining any historical
                    // manual part title that differs from the old file title.
                    tx.execute(
                        "UPDATE book_parts SET title=?1 WHERE file_id=?2 AND title=?3",
                        params![refreshed.title, id, previous.title],
                    )?;
                    if previous.cover.is_some() && previous.cover != refreshed.cover {
                        // Only artwork taken from the audio follows a rescan.
                        // A custom cover (or an explicitly cleared one) stays put.
                        let mut books = tx.prepare(
                            "SELECT b.id FROM books b JOIN book_parts p ON p.book_id=b.id
                             WHERE p.file_id=?1 AND b.cover=?2",
                        )?;
                        for book in
                            books.query_map(params![id, previous.cover], |r| r.get::<_, Id>(0))?
                        {
                            refresh_covers.insert(book?);
                        }
                    }
                    tx.execute(
                        "UPDATE media_files SET data=?1,size=?3,modified=?4,identity=?5 WHERE id=?2",
                        params![serde_json::to_string(&refreshed)?, id, super::stored_size(file.size)?, file.modified.unwrap_or(-1), file.identity],
                    )?;
                    if crate::source::document_identity(&file.uri).is_some() {
                        tx.execute(
                            "UPDATE media_files SET uri=?1,relative=CASE WHEN source_id=?3 THEN ?4 ELSE relative END WHERE id=?2",
                            params![file.uri, id, source, file.relative],
                        )?;
                    }
                    id
                } else {
                    tx.execute("INSERT INTO media_files(source_id,uri,relative,identity,size,modified,data) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![source,file.uri,file.relative,file.identity,super::stored_size(file.size)?,file.modified.unwrap_or(-1),serde_json::to_string(file)?])?;
                    tx.last_insert_rowid()
                };
                tx.execute("INSERT INTO source_files(source_id,file_id,relative) VALUES(?1,?2,?3) ON CONFLICT(source_id,file_id) DO NOTHING",params![source,id,file.relative])?;
                if crate::source::document_identity(&file.uri).is_some() {
                    // SAF IDs survive renames and moves within a source. Keep
                    // the path used for relocation in sync with the provider.
                    tx.execute(
                        "UPDATE source_files SET relative=?1 WHERE source_id=?2 AND file_id=?3",
                        params![file.relative, source, id],
                    )?;
                }
                if seen_files.insert(id) {
                    file_ids.push(id);
                }
            }
            {
                let mut matching = std::collections::BTreeSet::new();
                for file in &file_ids {
                    let mut stmt = tx.prepare("SELECT book_id FROM book_parts WHERE file_id=?1")?;
                    for id in stmt.query_map([file], |r| r.get::<_, Id>(0))? {
                        matching.insert(id?);
                    }
                }
                let existing: Option<Id> = tx
                    .query_row("SELECT id FROM books WHERE source_id=?1", [source], |r| {
                        r.get(0)
                    })
                    .optional()?;
                if let Some(id) = existing {
                    matching.insert(id);
                }
                if matching.len() > 1 {
                    bail!(tformat!(
                        "Папка пересекается с несколькими книгами. Импортируйте каждую книгу отдельно: {}",
                        draft.title
                    ));
                }
                let book = if let Some(id) = matching.first() {
                    *id
                } else {
                    tx.execute(
                        "INSERT INTO books(source_id,title,author,cover) VALUES(?1,?2,?3,?4)",
                        params![
                            source,
                            draft.title,
                            draft.author,
                            draft.files.iter().find_map(|f| f.cover.as_ref())
                        ],
                    )?;
                    tx.last_insert_rowid()
                };
                let mut ordinal: i64 = tx.query_row(
                    "SELECT COALESCE(MAX(ordinal)+1,0) FROM book_parts WHERE book_id=?1",
                    [book],
                    |r| r.get(0),
                )?;
                for file in file_ids {
                    let title: String =
                        tx.query_row("SELECT data FROM media_files WHERE id=?1", [file], |r| {
                            r.get(0)
                        })?;
                    let media: Media = serde_json::from_str(&title)?;
                    let inserted=tx.execute("INSERT INTO book_parts(book_id,file_id,ordinal,title) VALUES(?1,?2,?3,?4) ON CONFLICT(book_id,file_id) DO NOTHING",params![book,file,ordinal,media.title])?;
                    let part: Id = tx.query_row(
                        "SELECT id FROM book_parts WHERE book_id=?1 AND file_id=?2",
                        params![book, file],
                        |r| r.get(0),
                    )?;
                    tx.execute("DELETE FROM chapters WHERE part_id=?1", [part])?;
                    for c in media.chapters {
                        tx.execute(
                            "INSERT INTO chapters(part_id,title,start,end) VALUES(?1,?2,?3,?4)",
                            params![
                                part,
                                c.title,
                                i64::try_from(c.start)?,
                                c.end.map(i64::try_from).transpose()?
                            ],
                        )?;
                    }
                    if inserted > 0 {
                        ordinal += 1;
                    }
                }
            }
        }
        // Wait for every file to refresh before choosing the first surviving
        // embedded cover. This also handles removal of artwork from one part.
        for book in refresh_covers {
            let mut cover = None;
            let mut files = tx.prepare(
                "SELECT m.data FROM book_parts p JOIN media_files m ON m.id=p.file_id
                 WHERE p.book_id=?1 ORDER BY p.ordinal,p.id",
            )?;
            for data in files.query_map([book], |r| r.get::<_, String>(0))? {
                let media: Media = serde_json::from_str(&data?)?;
                if media.cover.is_some() {
                    cover = media.cover;
                    break;
                }
            }
            tx.execute(
                "UPDATE books SET cover=?1 WHERE id=?2",
                params![cover, book],
            )?;
        }
        tx.commit()?;
        self.load()
    }
}
