mod identity;
mod rescan;

use super::*;

#[test]
fn document_relocation_preserves_tags_completed_after_its_initial_snapshot() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let mut imported = draft("book", 1);
    imported.root_uri = "content://provider/tree/root/document/book".into();
    imported.files[0].uri = "content://provider/tree/root/document/recording".into();
    imported.files[0].identity = crate::source::document_identity(&imported.files[0].uri).unwrap();
    let before = store.import(vec![imported])?;
    // The relocation worker snapshots first, then validates a slow provider.
    let mut moved = before.media[0].clone();
    let tags = BookTags {
        year: Some(2026),
        genre: "Speech".into(),
    };
    assert!(store.update_sort_tags(&before.media[0], &tags)?);
    moved.uri = "content://provider/tree/destination/document/recording".into();
    moved.identity = crate::source::document_identity(&moved.uri).unwrap();
    moved.modified = Some(10);
    moved.size = Some(120);
    let tx = store.conn.transaction()?;
    crate::storage::sources::update_document_location(&tx, &moved)?;
    tx.commit()?;
    let after = store.load()?;
    let file = &after.media[0];
    assert_eq!(file.uri, moved.uri);
    assert_eq!((file.size, file.modified), (Some(120), Some(10)));
    assert_eq!(
        (file.year, file.genre.as_str(), file.sort_tags_read),
        (Some(2026), "Speech", true)
    );
    Ok(())
}
