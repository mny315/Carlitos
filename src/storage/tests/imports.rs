use super::*;

#[test]
fn rescanning_refreshes_embedded_covers_and_preserves_custom_covers() -> Result<()> {
    for custom in [false, true] {
        let temp = tempfile::tempdir()?;
        let mut store = Store::open(&temp.path().join("db"))?;
        let mut imported = draft("artwork", 2);
        imported.files[0].cover = Some("old.png".into());
        imported.files[1].cover = Some("fallback.png".into());
        let before = store.import(vec![imported.clone()])?;
        if custom {
            store.edit_book(
                before.books[0].id,
                before.books[0].title.clone(),
                before.books[0].author.clone(),
                before.parts.iter().map(|part| part.id).collect(),
                Some("custom.png".into()),
            )?;
        }
        imported.files[0].cover = Some("corrected.png".into());
        let refreshed = store.import(vec![imported.clone()])?;
        assert_eq!(
            refreshed.books[0].cover.as_deref(),
            Some(if custom {
                "custom.png"
            } else {
                "corrected.png"
            })
        );
        imported.files[0].cover = None;
        let refreshed = store.import(vec![imported])?;
        assert_eq!(
            refreshed.books[0].cover.as_deref(),
            Some(if custom { "custom.png" } else { "fallback.png" })
        );
        assert_eq!(refreshed.parts[0].id, before.parts[0].id);
    }
    Ok(())
}

#[test]
fn rescanning_refreshes_automatic_part_titles_and_keeps_custom_titles() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let mut original = draft("retagged", 2);
    let before = store.import(vec![original.clone()])?;
    store.conn.execute(
        "UPDATE book_parts SET title='My chapter' WHERE id=?1",
        [before.parts[1].id],
    )?;
    original.files[0].title = "New tag title".into();
    original.files[1].title = "Another tag title".into();
    let after = store.import(vec![original])?;
    assert_eq!(after.parts[0].title, "New tag title");
    assert_eq!(after.parts[1].title, "My chapter");
    assert_eq!(after.parts[0].id, before.parts[0].id);
    Ok(())
}

#[test]
fn rescan_refreshes_file_characteristics_without_replacing_parts() -> Result<()> {
    let t = tempfile::tempdir()?;
    let mut store = Store::open(&t.path().join("db"))?;
    let mut imported = draft("updated", 1);
    imported.files[0].size = Some(123);
    imported.files[0].modified = Some(10);
    imported.files[0].chapters.push(Chapter {
        title: "Old recording".into(),
        start: 0,
        end: Some(60_000),
    });
    let before = store.import(vec![imported.clone()])?;
    imported.files[0].size = Some(456);
    imported.files[0].modified = Some(20);
    imported.files[0].identity = "updated_inode".into();
    imported.files[0].duration = Some(72_000);
    imported.files[0].chapters.clear();
    let after = store.import(vec![imported])?;
    assert_eq!(after.parts[0].id, before.parts[0].id);
    assert_eq!(after.media[0].id, before.media[0].id);
    assert_eq!(after.media[0].size, Some(456));
    assert_eq!(after.media[0].modified, Some(20));
    assert_eq!(after.media[0].identity, "updated_inode");
    assert_eq!(after.media[0].duration, Some(72_000));
    assert!(after.media[0].chapters.is_empty());
    assert_eq!(
        store
            .conn
            .query_row("SELECT COUNT(*) FROM chapters", [], |r| r.get::<_, i64>(0))?,
        0
    );
    Ok(())
}

#[test]
fn custom_cover_and_tag_backfill_preserve_book_edits_and_progress() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp.path().join("db");
    let mut store = Store::open(&path)?;
    let mut library = store.import(vec![draft("book", 2)])?;
    let book = library.books[0].id;
    let order: Vec<_> = library
        .book_parts(book)
        .iter()
        .rev()
        .map(|p| p.id)
        .collect();
    library.session.current = Some(Target::Book(order[0]));
    library.update_progress(20_000, false);
    store.save(&library.session, &library.progress)?;
    store.edit_book(
        book,
        "My title".into(),
        "My author".into(),
        order.clone(),
        Some("/custom/cover.png".into()),
    )?;
    let tags = BookTags {
        year: Some(1997),
        genre: "Science fiction".into(),
    };
    assert!(store.update_sort_tags(&library.media[0], &tags)?);
    assert!(!store.update_sort_tags(&library.media[0], &BookTags::default())?);
    let after = Store::open(&path)?.load()?;
    assert_eq!(after.books[0].cover.as_deref(), Some("/custom/cover.png"));
    assert_eq!(after.book_tags()[&book].year, Some(1997));
    assert_eq!(after.book_tags()[&book].genre, "Science fiction");
    assert_eq!(after.progress[0].position, 20_000);
    let mut rescanned = draft("book", 2);
    rescanned.files[0].year = Some(2002);
    rescanned.files[0].sort_tags_read = true;
    let after = store.import(vec![rescanned])?;
    assert_eq!(after.books[0].title, "My title");
    assert_eq!(after.books[0].cover.as_deref(), Some("/custom/cover.png"));
    assert!(!store.update_sort_tags(&library.media[0], &tags)?);
    assert_eq!(store.load()?.book_tags()[&book].year, Some(2002));
    assert_eq!(
        after
            .book_parts(book)
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        order
    );
    store.remove_book(book)?;
    store.remove_source(library.sources[0].id)?;
    assert!(!store.update_sort_tags(&library.media[0], &tags)?);
    Ok(())
}

#[test]
fn overlapping_import_and_transaction_rollback() -> Result<()> {
    let t = tempfile::tempdir()?;
    let mut s = Store::open(&t.path().join("db"))?;
    s.import(vec![draft("one", 1), draft("two", 1)])?;
    let mut overlap = draft("combined", 0);
    overlap.files = vec![
        draft("one", 1).files.remove(0),
        draft("two", 1).files.remove(0),
    ];
    assert!(s.import(vec![draft("must_rollback", 2), overlap]).is_err());
    let l = s.load()?;
    assert_eq!(l.sources.len(), 2);
    assert_eq!(l.media.len(), 2);
    Ok(())
}
