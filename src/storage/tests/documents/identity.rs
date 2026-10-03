use super::super::*;

#[test]
fn document_preview_rejects_ambiguous_names_and_ids_across_aliased_roots() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let uri = "content://provider/tree/root/document/first";
    let first = Draft {
        root_uri: "content://provider/tree/root/document/book".into(),
        title: "Book".into(),
        author: String::new(),
        include: true,
        files: vec![Media {
            uri: uri.into(),
            identity: crate::source::document_identity(uri).unwrap(),
            relative: "chapter.wav".into(),
            ..Default::default()
        }],
    };
    for same_document in [false, true] {
        let mut conflict = first.clone();
        conflict.root_uri = "content://provider/tree/book/document/book".into();
        if same_document {
            conflict.files[0].relative = "another.wav".into();
        } else {
            conflict.files[0].uri = "content://provider/tree/book/document/second".into();
            conflict.files[0].identity =
                crate::source::document_identity(&conflict.files[0].uri).unwrap();
        }
        assert!(store.import(vec![first.clone(), conflict.clone()]).is_err());
        assert!(store.load()?.books.is_empty());
        conflict.include = false;
        assert_eq!(store.import(vec![first.clone(), conflict])?.media.len(), 1);
        let source = store.load()?.sources[0].id;
        store.remove_source(source)?;
    }
    Ok(())
}

#[test]
fn new_document_can_reuse_an_absent_documents_name_without_inheriting_progress() -> Result<()> {
    for overlapping in [false, true] {
        let temp = tempfile::tempdir()?;
        let database = temp.path().join("db");
        let mut store = Store::open(&database)?;
        let recording = |id: &str, name: &str| {
            let uri = format!("content://provider/tree/root/document/{id}");
            Media {
                identity: crate::source::document_identity(&uri).unwrap(),
                uri,
                relative: name.into(),
                title: id.into(),
                duration: Some(60_000),
                ..Default::default()
            }
        };
        let mut draft = Draft {
            root_uri: "content://provider/tree/root/document/book".into(),
            title: "Book".into(),
            author: String::new(),
            include: true,
            files: vec![recording("old", "1.wav"), recording("stable", "2.wav")],
        };
        let mut before = store.import(vec![draft.clone()])?;
        let part = before.parts[0].id;
        before.session.current = Some(Target::Book(part));
        before.update_progress(12_345, false);
        store.save(&before.session, &before.progress)?;
        if overlapping {
            draft.root_uri = "content://provider/tree/root/document/parent".into();
            for file in &mut draft.files {
                file.relative = format!("book/{}", file.relative);
            }
            store.import(vec![draft.clone()])?;
        }
        let mut old = draft.files[0].clone();
        draft.files[0] = recording("replacement", &old.relative);
        let after = store.import(vec![draft.clone()])?;
        assert_eq!(
            (after.books.len(), after.parts.len(), after.media.len()),
            (1, 3, 3)
        );
        assert_eq!(after.session.current, Some(Target::Book(part)));
        assert_eq!(after.session.position, 12_345);
        assert_eq!(after.progress[0].part_id, part);
        assert_eq!(after.progress[0].position, 12_345);
        let replacement = after
            .media
            .iter()
            .find(|file| file.identity == draft.files[0].identity)
            .unwrap();
        assert_ne!(replacement.id, before.parts[0].file_id);
        assert_eq!(after.media(before.parts[0].file_id).unwrap().uri, old.uri);
        assert_eq!(store.import(vec![draft.clone()])?.parts.len(), 3);
        // If the old ID returns under a new name, it regains its existing part.
        old.relative = if overlapping {
            "book/returned.wav"
        } else {
            "returned.wav"
        }
        .into();
        draft.files.push(old);
        assert_eq!(store.import(vec![draft])?.parts.len(), 3);
        drop(store);
        let restored = Store::open(&database)?.load()?;
        assert_eq!(restored.session.current, Some(Target::Book(part)));
        assert_eq!(restored.progress[0].position, 12_345);
    }
    Ok(())
}

#[test]
fn saf_overlapping_grants_upgrade_legacy_identity_and_keep_progress() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let database = temp.path().join("library.sqlite3");
    let mut store = Store::open(&database)?;
    let single = "content://provider/document/disk%3Abook%2F01.wav";
    let tree = "content://provider/tree/disk%3A/document/disk%3Abook%2F01.wav";
    let root = "content://provider/tree/disk%3A/document/disk%3Abook";
    let file = Media {
        uri: single.into(),
        identity: format!("document:{single}"),
        relative: "01.wav".into(),
        title: "Part 1".into(),
        ..Default::default()
    };
    let mut original = store.import(vec![Draft {
        root_uri: single.into(),
        title: "Book".into(),
        author: String::new(),
        files: vec![file.clone()],
        include: true,
    }])?;
    original.session.current = Some(Target::Book(original.parts[0].id));
    original.update_progress(12345, false);
    store.save(&original.session, &original.progress)?;
    let mut file = file;
    file.uri = tree.into();
    file.identity = crate::source::document_identity(tree).unwrap();
    let draft = Draft {
        root_uri: root.into(),
        title: "Folder".into(),
        author: String::new(),
        files: vec![file],
        include: true,
    };
    let next = store.import(vec![draft.clone()])?;
    assert_eq!(
        (next.media.len(), next.books.len(), next.parts.len()),
        (1, 1, 1)
    );
    assert_eq!(next.parts[0].id, original.parts[0].id);
    assert_eq!(next.media[0].id, original.media[0].id);
    assert_eq!(next.media[0].uri, tree);
    assert_eq!(next.session.position, 12345);
    assert_eq!(next.progress[0].position, 12345);
    assert!(
        next.sources
            .iter()
            .all(|s| next.source_book_counts()[&s.id] == 1)
    );
    let mut alternate = draft;
    alternate.root_uri = "content://provider/tree/disk%3Abook/document/disk%3Abook".into();
    alternate.files[0].uri =
        "content://provider/tree/disk%3Abook/document/disk%3Abook%2F01.wav".into();
    let again = store.import(vec![alternate])?;
    assert_eq!(again.sources.len(), next.sources.len());
    assert_eq!(again.books[0].id, original.books[0].id);
    let folder = again.sources.iter().find(|s| s.uri != single).unwrap().id;
    let removed = store.remove_source(folder)?;
    assert_eq!(removed.media[0].uri, single);
    drop(store);
    let restored = Store::open(&database)?.load()?;
    assert_eq!(restored.session.position, 12345);
    assert_eq!(restored.media[0].size, None);
    assert_eq!(restored.media[0].modified, None);
    Ok(())
}

#[test]
fn unknown_document_properties_do_not_prove_recording_unchanged() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let uri = "content://provider/document/opaque";
    let mut file = Media {
        uri: uri.into(),
        identity: crate::source::document_identity(uri).unwrap(),
        relative: "part.wav".into(),
        title: "Part".into(),
        chapters: vec![Chapter {
            title: "Old".into(),
            start: 0,
            end: None,
        }],
        ..Default::default()
    };
    let draft = |file| Draft {
        root_uri: uri.into(),
        title: "Book".into(),
        author: String::new(),
        files: vec![file],
        include: true,
    };
    store.import(vec![draft(file.clone())])?;
    file.chapters.clear();
    let loaded = store.import(vec![draft(file)])?;
    assert!(loaded.media[0].chapters.is_empty());
    let properties: (i64, i64) =
        store
            .conn
            .query_row("SELECT size,modified FROM media_files", [], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })?;
    assert_eq!(properties, (-1, -1));
    // Unknown and a known empty file must remain distinguishable after restart.
    assert_eq!(loaded.media[0].size, None);
    Ok(())
}
