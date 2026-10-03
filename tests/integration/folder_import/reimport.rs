use super::*;

#[test]
fn reimport_splits_legacy_book_without_losing_order_chapters_or_progress() -> anyhow::Result<()> {
    for root_audio in [false, true] {
        let temp = tempdir()?;
        let root = temp.path().join("Audiobooks");
        for file in [
            "Автор/Книга 1/1.wav",
            "Автор/Книга 1/2.wav",
            "Книга 2/1.wav",
        ] {
            audio(&root, file)?;
        }
        if root_audio {
            audio(&root, "Отдельная запись.wav")?;
        }
        let db_path = temp.path().join("db");
        let mut store = Store::open(&db_path)?;
        let mut combined = scan(vec![root.clone()], ImportMode::Book, ScanControl::default())?;
        combined.drafts[0].files[0].chapters.push(Chapter {
            title: "Opening".into(),
            start: 0,
            end: Some(100),
        });
        let library = store.import(combined.drafts)?;
        assert_eq!(library.books.len(), 1);
        let book = library.books[0].id;
        let ids: Vec<_> = library
            .book_parts(book)
            .iter()
            .rev()
            .map(|p| p.id)
            .collect();
        let mut library =
            store.edit_book(book, "Combined".into(), String::new(), ids.clone(), None)?;
        let part = library
            .parts
            .iter()
            .find(|p| {
                library.media(p.file_id).unwrap().uri
                    == file_uri(&root.join("Автор/Книга 1/2.wav")).unwrap()
            })
            .unwrap()
            .id;
        library.session.current = Some(Target::Book(part));
        library.update_progress(50, false);
        store.save(&library.session, &library.progress)?;
        let result = scan(
            vec![root.clone()],
            ImportMode::Books,
            ScanControl::default(),
        )?;
        let mut after = store.import(result.drafts.clone())?;
        assert_eq!(
            after.media.iter().map(|m| m.chapters.len()).sum::<usize>(),
            1
        );
        assert_eq!(after.books.len(), if root_audio { 3 } else { 2 });
        assert_eq!(after.sources.len(), after.books.len());
        assert_eq!(after.media.len(), library.media.len());
        assert_eq!(after.parts.len(), library.parts.len());
        assert_eq!(after.progress[0].part_id, part);
        assert_eq!(after.progress[0].position, 50);
        assert_eq!(after.progress[0].book_id, after.part(part).unwrap().book_id);
        assert_eq!(after.session.current, Some(Target::Book(part)));
        for book in &after.books {
            let parts = after.book_parts(book.id);
            let expected: Vec<_> = ids
                .iter()
                .copied()
                .filter(|id| parts.iter().any(|p| p.id == *id))
                .collect();
            assert_eq!(parts.iter().map(|p| p.id).collect::<Vec<_>>(), expected);
            let source = after
                .sources
                .iter()
                .find(|s| s.id == book.source_id)
                .unwrap();
            let source_root = local_path(&source.uri).unwrap();
            for part in parts {
                let file = after.media(part.file_id).unwrap();
                assert_eq!(
                    local_path(&file.uri).unwrap(),
                    source_root.join(&file.relative)
                );
            }
        }
        after.update_progress(75, false);
        store.save(&after.session, &after.progress)?;
        let again = store.import(result.drafts)?;
        assert_eq!(
            again.parts.iter().map(|p| p.id).collect::<Vec<_>>(),
            after.parts.iter().map(|p| p.id).collect::<Vec<_>>()
        );
        drop(store);
        let restored = Store::open(&db_path)?.load()?;
        assert_eq!(
            restored
                .media
                .iter()
                .map(|m| m.chapters.len())
                .sum::<usize>(),
            1
        );
        assert_eq!(restored.progress[0].position, 75);
        let conn = rusqlite::Connection::open(&db_path)?;
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM pragma_foreign_key_check", [], |r| r
                .get::<_, i64>(
                0
            ))?,
            0
        );
        assert_eq!(
            conn.query_row("SELECT COUNT(*) FROM chapters", [], |r| r.get::<_, i64>(0))?,
            1
        );
    }
    Ok(())
}

#[test]
fn incomplete_reimport_keeps_unselected_parts_and_progress() -> anyhow::Result<()> {
    let temp = tempdir()?;
    for file in ["One/1.wav", "Two/1.wav"] {
        audio(temp.path(), file)?;
    }
    let mut store = Store::open(&temp.path().join("db"))?;
    let combined = scan(
        vec![temp.path().into()],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    let before = store.import(combined.drafts)?;
    let mut result = scan(
        vec![temp.path().into()],
        ImportMode::Books,
        ScanControl::default(),
    )?;
    result.drafts[1].include = false;
    let after = store.import(result.drafts)?;
    assert_eq!(after.books.len(), 1);
    assert_eq!(
        after.parts.iter().map(|p| p.id).collect::<Vec<_>>(),
        before.parts.iter().map(|p| p.id).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn reimporting_individual_files_from_one_folder_preserves_the_book() -> anyhow::Result<()> {
    let temp = tempdir()?;
    let paths: Vec<_> = ["1.wav", "2.wav"]
        .iter()
        .map(|name| temp.path().join(name))
        .collect();
    for name in ["1.wav", "2.wav"] {
        audio(temp.path(), name)?;
    }
    let mut store = Store::open(&temp.path().join("db"))?;
    let drafts = scan(paths.clone(), ImportMode::Book, ScanControl::default())?.drafts;
    let before = store.import(drafts)?;
    let book = before.books[0].id;
    let order: Vec<_> = before.book_parts(book).iter().rev().map(|p| p.id).collect();
    store.edit_book(
        book,
        "Saved title".into(),
        "Saved author".into(),
        order.clone(),
        None,
    )?;
    let drafts = scan(paths, ImportMode::Book, ScanControl::default())?.drafts;
    let after = store.import(drafts)?;
    assert_eq!(after.books.len(), 1);
    assert_eq!(after.books[0].title, "Saved title");
    assert_eq!(after.books[0].author, "Saved author");
    assert_eq!(
        after
            .book_parts(book)
            .iter()
            .map(|p| p.id)
            .collect::<Vec<_>>(),
        order
    );
    assert_eq!(
        after
            .book_parts(book)
            .iter()
            .map(|p| p.ordinal)
            .collect::<Vec<_>>(),
        [0, 1]
    );
    Ok(())
}
