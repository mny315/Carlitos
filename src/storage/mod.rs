#[cfg(target_os = "android")]
mod documents;
mod import;
mod migrations;
mod sources;
#[cfg(test)]
mod tests;

use crate::library::*;
use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension, params};

pub struct Store {
    conn: Connection,
}
#[derive(Clone, Debug)]
pub struct Draft {
    pub root_uri: String,
    pub title: String,
    pub author: String,
    pub files: Vec<Media>,
    pub include: bool,
}

impl Store {
    pub fn load(&self) -> Result<Library> {
        // A background relocation can commit between SELECTs. Keep every
        // table, including the session, on the same SQLite read snapshot.
        let snapshot = self.conn.unchecked_transaction()?;
        let mut lib = Library::default();
        let mut stmt = self
            .conn
            .prepare("SELECT id,uri FROM sources WHERE kind='book' ORDER BY id")?;
        lib.sources = stmt
            .query_map([], |r| {
                Ok(Source {
                    id: r.get(0)?,
                    uri: r.get(1)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        lib.source_files = self
            .conn
            .prepare("SELECT source_id,file_id FROM source_files")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut stmt = self.conn.prepare("SELECT id,source_id,uri,relative,identity,size,modified,data FROM media_files WHERE id IN (SELECT file_id FROM book_parts) OR id IN (SELECT sf.file_id FROM source_files sf JOIN sources s ON sf.source_id=s.id WHERE s.kind='book') ORDER BY id")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                r.get::<_, String>(7)?,
            ))
        })?;
        for row in rows {
            let (id, source_id, uri, relative, identity, size, modified, json) = row?;
            let mut m: Media = serde_json::from_str(&json)?;
            m.id = id;
            m.source_id = source_id;
            m.uri = uri;
            m.relative = relative;
            m.identity = identity;
            m.size = u64::try_from(size).ok();
            m.modified = (modified >= 0).then_some(modified);
            // The one-file Android prototype stored placeholders, not facts.
            if m.identity.starts_with("document:content:") && size == 0 && modified == 0 {
                m.size = None;
                m.modified = None;
            }
            lib.media.push(m);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT id,source_id,title,author,cover FROM books ORDER BY title,id")?;
        lib.books = stmt
            .query_map([], |r| {
                Ok(Book {
                    id: r.get(0)?,
                    source_id: r.get(1)?,
                    title: r.get(2)?,
                    author: r.get(3)?,
                    cover: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        let mut stmt = self.conn.prepare(
            "SELECT id,book_id,file_id,title,ordinal FROM book_parts ORDER BY book_id,ordinal",
        )?;
        lib.parts = stmt
            .query_map([], |r| {
                Ok(Part {
                    id: r.get(0)?,
                    book_id: r.get(1)?,
                    file_id: r.get(2)?,
                    title: r.get(3)?,
                    ordinal: r.get::<_, i64>(4)? as usize,
                })
            })?
            .collect::<Result<_, _>>()?;
        let mut stmt = self
            .conn
            .prepare("SELECT book_id,part_id,position,completed,updated FROM book_progress")?;
        lib.progress = stmt
            .query_map([], |r| {
                Ok(Progress {
                    book_id: r.get(0)?,
                    part_id: r.get(1)?,
                    position: r.get::<_, i64>(2)? as u64,
                    completed: r.get(3)?,
                    updated: r.get(4)?,
                })
            })?
            .collect::<Result<_, _>>()?;
        lib.session = load_session(&self.conn)?;
        if lib.session.current.is_some() && lib.active_file().is_none() {
            lib.session.current = None;
            lib.session.position = 0;
        }
        lib.session.volume = if lib.session.volume.is_finite() {
            lib.session.volume.clamp(0., 1.)
        } else {
            0.7
        };
        snapshot.commit()?;
        Ok(lib)
    }
    pub fn save(&mut self, session: &Session, progress: &[Progress]) -> Result<()> {
        let tx = self.conn.transaction()?;
        save_session(&tx, session)?;
        {
            // A periodic checkpoint may already be queued when a source/book is removed.
            // Discard entries for deleted books or parts while retaining the
            // composite FK check against a part from the wrong surviving book.
            // Prepare once and leave unchanged books untouched: a checkpoint
            // normally advances only one recording in a large library.
            let mut checkpoint = tx.prepare_cached(
                "INSERT INTO book_progress(book_id,part_id,position,completed,updated)
                 SELECT ?1,?2,?3,?4,?5
                 WHERE EXISTS(SELECT 1 FROM books WHERE id=?1)
                   AND EXISTS(SELECT 1 FROM book_parts WHERE id=?2)
                 ON CONFLICT(book_id) DO UPDATE SET
                   part_id=excluded.part_id,position=excluded.position,
                   completed=excluded.completed,updated=excluded.updated
                 WHERE book_progress.part_id!=excluded.part_id
                    OR book_progress.position!=excluded.position
                    OR book_progress.completed!=excluded.completed
                    OR book_progress.updated!=excluded.updated",
            )?;
            for p in progress {
                checkpoint.execute(params![
                    p.book_id,
                    p.part_id,
                    i64::try_from(p.position)?,
                    p.completed,
                    p.updated
                ])?;
            }
        }
        tx.commit()?;
        Ok(())
    }
    pub fn reset_book_progress(&mut self, id: Id) -> Result<()> {
        let tx = self.conn.transaction()?;
        let mut session = load_session(&tx)?;
        if let Some(Target::Book(part)) = session.current
            && tx
                .query_row("SELECT book_id FROM book_parts WHERE id=?1", [part], |r| {
                    r.get::<_, Id>(0)
                })
                .optional()?
                == Some(id)
        {
            session.current = None;
            session.position = 0;
        }
        tx.execute("DELETE FROM book_progress WHERE book_id=?1", [id])?;
        save_session(&tx, &session)?;
        tx.commit()?;
        Ok(())
    }
    pub fn edit_book(
        &mut self,
        id: Id,
        title: String,
        author: String,
        order: Vec<Id>,
        cover: Option<String>,
    ) -> Result<Library> {
        let tx = self.conn.transaction()?;
        let mut stmt = tx.prepare("SELECT id FROM book_parts WHERE book_id=?1")?;
        let expected: std::collections::BTreeSet<Id> = stmt
            .query_map([id], |r| r.get(0))?
            .collect::<Result<_, _>>()?;
        drop(stmt);
        if expected != order.iter().copied().collect() || expected.len() != order.len() {
            bail!(crate::i18n::tr(
                "Список частей изменился; откройте книгу заново"
            ));
        }
        tx.execute(
            "UPDATE books SET title=?1,author=?2,cover=?4 WHERE id=?3",
            params![title, author, id, cover],
        )?;
        for (i, p) in order.iter().enumerate() {
            tx.execute(
                "UPDATE book_parts SET ordinal=?1 WHERE id=?2 AND book_id=?3",
                params![i as i64, p, id],
            )?;
        }
        tx.commit()?;
        self.load()
    }
    pub fn update_sort_tags(&mut self, original: &Media, tags: &BookTags) -> Result<bool> {
        let data: Option<String> = self.conn.query_row(
            "SELECT data FROM media_files WHERE id=?1 AND uri=?2 AND identity=?3 AND size=?4 AND modified=?5",
            params![original.id, original.uri, original.identity, stored_size(original.size)?, original.modified.unwrap_or(-1)],
            |r| r.get(0),
        ).optional()?;
        let Some(data) = data else {
            return Ok(false);
        };
        let mut media: Media = serde_json::from_str(&data)?;
        if media.sort_tags_read {
            return Ok(false);
        }
        media.year = tags.year;
        media.genre = tags.genre.clone();
        media.sort_tags_read = true;
        Ok(self.conn.execute(
            "UPDATE media_files SET data=?1 WHERE id=?2 AND data=?3 AND uri=?4",
            params![
                serde_json::to_string(&media)?,
                original.id,
                data,
                original.uri
            ],
        )? > 0)
    }
    pub fn remove_book(&mut self, id: Id) -> Result<Library> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM books WHERE id=?1", [id])?;
        save_session(&tx, &load_session(&tx)?)?;
        tx.commit()?;
        self.load()
    }
}

fn load_session(conn: &Connection) -> Result<Session> {
    let state: Option<String> = conn
        .query_row("SELECT data FROM session_state WHERE id=1", [], |r| {
            r.get(0)
        })
        .optional()?;
    state.map_or_else(
        || Ok(Session::default()),
        |state| {
            serde_json::from_str(&state)
                .context(crate::i18n::tr("Повреждено сохранённое состояние"))
        },
    )
}

fn save_session(conn: &Connection, session: &Session) -> Result<()> {
    let mut session = session.clone();
    // JSON has no foreign key: clear removed targets before SQLite can reuse
    // their IDs for a later import, including saves queued before a deletion.
    if let Some(Target::Book(id)) = session.current
        && !conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM book_parts WHERE id=?1)",
            [id],
            |r| r.get::<_, bool>(0),
        )?
    {
        session.current = None;
        session.position = 0;
    }
    conn.execute(
        "INSERT INTO session_state(id,data) VALUES(1,?1)
         ON CONFLICT(id) DO UPDATE SET data=excluded.data
         WHERE session_state.data!=excluded.data",
        [serde_json::to_string(&session)?],
    )?;
    Ok(())
}

// Existing NOT NULL columns retain their schema; -1 explicitly means unknown.
fn stored_size(size: Option<u64>) -> Result<i64> {
    Ok(size.map(i64::try_from).transpose()?.unwrap_or(-1))
}
