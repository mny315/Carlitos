use crate::library::{Id, Media, local_path};
use anyhow::{Context, Result, bail};
use rusqlite::{OptionalExtension, params};

/// A device/inode pair is only an identity while its original path still
/// resolves to that file. Filesystems may reuse inodes after a deletion.
pub(super) fn existing_file(tx: &rusqlite::Transaction<'_>, file: &Media) -> Result<Option<Id>> {
    let by_uri: Option<Id> = tx
        .query_row(
            "SELECT id FROM media_files WHERE uri=?1",
            [&file.uri],
            |r| r.get(0),
        )
        .optional()?;
    let mut by_identity: Option<(Id, String)> = tx
        .query_row(
            "SELECT id,uri FROM media_files WHERE identity=?1",
            [&file.identity],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if by_identity.is_none() && crate::source::document_identity(&file.uri).is_some() {
        let legacy: Vec<(Id, String)> = tx
            .prepare("SELECT id,uri FROM media_files WHERE identity LIKE 'document:content:%'")?
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        by_identity = legacy.into_iter().find(|(_, uri)| {
            crate::source::document_identity(uri).as_ref() == Some(&file.identity)
        });
    }
    let Some((id, uri)) = by_identity else {
        return Ok(by_uri);
    };
    if by_uri == Some(id) {
        return Ok(by_uri);
    }
    let same_file = if let Some(identity) = crate::source::document_identity(&uri) {
        identity == file.identity
    } else if let Some(path) = local_path(&uri) {
        match crate::import::identity(&path) {
            Ok(identity) => identity == file.identity,
            Err(e)
                if e.downcast_ref::<std::io::Error>()
                    .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound) =>
            {
                false
            }
            Err(e) => {
                return Err(e).with_context(|| {
                    tformat!("Не удалось проверить исходный файл {}", path.display())
                });
            }
        }
    } else {
        false
    };
    if same_file {
        if by_uri.is_some() {
            bail!(tformat!(
                "Файл совпадает с двумя записями библиотеки. Проверьте источники: {}",
                file.uri
            ));
        }
        return Ok(Some(id));
    }
    // Keep the unavailable record, its parts and progress. Its retired identity
    // must not prevent importing a different file that reused the inode.
    tx.execute(
        "UPDATE media_files SET identity=?1 WHERE id=?2",
        params![format!("retired:{id}:{}", file.identity), id],
    )?;
    Ok(by_uri)
}
