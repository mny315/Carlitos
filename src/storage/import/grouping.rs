use crate::{
    library::{Id, local_path},
    storage::Draft,
};
use anyhow::Result;
use rusqlite::params;

/// Reimporting a collection can replace a legacy combined book with the
/// reviewed groups. Only split when every existing part has an unambiguous
/// destination; preserve part IDs, manual order, chapters and the checkpoint.
pub(super) fn split_combined_books(tx: &rusqlite::Transaction<'_>, drafts: &[Draft]) -> Result<()> {
    use std::collections::{BTreeMap, HashMap, HashSet};
    let mut destinations = HashMap::new();
    for (index, draft) in drafts.iter().enumerate().filter(|(_, d)| d.include) {
        for file in &draft.files {
            destinations
                .entry(file.uri.as_str())
                .and_modify(|destination| *destination = None)
                .or_insert(Some(index));
        }
    }
    let books: Vec<(Id, Id, String)> = tx
        .prepare("SELECT b.id,b.source_id,s.uri FROM books b JOIN sources s ON s.id=b.source_id")?
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    for (book, old_source, uri) in books {
        let Some(root) = local_path(&uri) else {
            continue;
        };
        let parts: Vec<(Id, Id, String)> = tx
            .prepare("SELECT p.id,p.file_id,m.uri FROM book_parts p JOIN media_files m ON m.id=p.file_id WHERE p.book_id=?1 ORDER BY p.ordinal,p.id")?
            .query_map([book], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?;
        let mut groups = BTreeMap::<usize, Vec<_>>::new();
        for (part, file, uri) in &parts {
            if let Some(Some(index)) = destinations.get(uri.as_str())
                && local_path(&drafts[*index].root_uri).is_some_and(|p| p.starts_with(&root))
            {
                groups.entry(*index).or_default().push((*part, *file, uri));
            }
        }
        let roots: HashSet<_> = groups.keys().map(|i| &drafts[*i].root_uri).collect();
        if groups.len() < 2
            || roots.len() != groups.len()
            || groups.values().map(Vec::len).sum::<usize>() != parts.len()
        {
            continue;
        }
        // A grouping must not fold a different existing book into this one.
        let mut overlaps = false;
        for index in groups.keys() {
            for file in &drafts[*index].files {
                overlaps |= tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM book_parts p JOIN media_files m ON m.id=p.file_id WHERE m.uri=?1 AND p.book_id<>?2)",
                    params![file.uri, book], |r| r.get::<_, bool>(0),
                )?;
            }
        }
        if overlaps {
            continue;
        }
        // The checkpoint's composite FK spans the two updates below.
        tx.execute_batch("PRAGMA defer_foreign_keys=ON")?;
        for (index, parts) in groups {
            let draft = &drafts[index];
            tx.execute(
                "INSERT INTO sources(uri,kind) VALUES(?1,'book') ON CONFLICT(uri,kind) DO NOTHING",
                [&draft.root_uri],
            )?;
            let source: Id = tx.query_row(
                "SELECT id FROM sources WHERE uri=?1 AND kind='book'",
                [&draft.root_uri],
                |r| r.get(0),
            )?;
            tx.execute(
                "INSERT INTO books(source_id,title,author,cover) VALUES(?1,?2,?3,?4) ON CONFLICT(source_id) DO UPDATE SET title=excluded.title,author=excluded.author,cover=excluded.cover",
                params![source, draft.title, draft.author, draft.files.iter().find_map(|f| f.cover.as_ref())],
            )?;
            let destination: Id =
                tx.query_row("SELECT id FROM books WHERE source_id=?1", [source], |r| {
                    r.get(0)
                })?;
            let relatives: HashMap<_, _> = draft
                .files
                .iter()
                .map(|f| (f.uri.as_str(), f.relative.as_str()))
                .collect();
            for (ordinal, (part, file, uri)) in parts.into_iter().enumerate() {
                tx.execute(
                    "UPDATE book_parts SET book_id=?1,ordinal=?2 WHERE id=?3",
                    params![destination, ordinal as i64, part],
                )?;
                tx.execute(
                    "UPDATE book_progress SET book_id=?1 WHERE book_id=?2 AND part_id=?3",
                    params![destination, book, part],
                )?;
                let relative = relatives[uri.as_str()];
                tx.execute("INSERT INTO source_files(source_id,file_id,relative) VALUES(?1,?2,?3) ON CONFLICT(source_id,file_id) DO UPDATE SET relative=excluded.relative", params![source, file, relative])?;
                if source != old_source {
                    tx.execute("UPDATE media_files SET source_id=?1,relative=?2 WHERE id=?3 AND source_id=?4", params![source, relative, file, old_source])?;
                    tx.execute(
                        "DELETE FROM source_files WHERE source_id=?1 AND file_id=?2",
                        params![old_source, file],
                    )?;
                }
            }
        }
        tx.execute("DELETE FROM books WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM book_parts WHERE book_id=?1)", [book])?;
        tx.execute("DELETE FROM sources WHERE id=?1 AND NOT EXISTS(SELECT 1 FROM books WHERE source_id=?1) AND NOT EXISTS(SELECT 1 FROM media_files WHERE source_id=?1) AND NOT EXISTS(SELECT 1 FROM source_files WHERE source_id=?1)", [old_source])?;
    }
    Ok(())
}
