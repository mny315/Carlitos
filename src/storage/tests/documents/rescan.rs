use super::super::*;

#[test]
fn document_rescan_can_swap_names_in_owned_and_overlapping_sources() -> Result<()> {
    for overlapping in [false, true] {
        let temp = tempfile::tempdir()?;
        let mut store = Store::open(&temp.path().join("db"))?;
        let mut draft = Draft {
            root_uri: "content://provider/tree/root/document/book".into(),
            title: "Book".into(),
            author: String::new(),
            include: true,
            files: ["a", "b"]
                .into_iter()
                .map(|id| {
                    let uri = format!("content://provider/tree/root/document/{id}");
                    Media {
                        identity: crate::source::document_identity(&uri).unwrap(),
                        uri,
                        relative: format!("{id}.wav"),
                        title: id.into(),
                        ..Default::default()
                    }
                })
                .collect(),
        };
        let mut before = store.import(vec![draft.clone()])?;
        before.session.current = Some(Target::Book(before.parts[0].id));
        before.update_progress(12_345, false);
        store.save(&before.session, &before.progress)?;
        if overlapping {
            draft.root_uri = "content://provider/tree/root/document/parent".into();
            for file in &mut draft.files {
                file.relative = format!("book/{}", file.relative);
            }
            store.import(vec![draft.clone()])?;
        }
        let first = draft.files[0].relative.clone();
        draft.files[0].relative = draft.files[1].relative.clone();
        draft.files[1].relative = first;
        let snapshot = serde_json::to_value(store.load()?.media)?;
        let links = |store: &Store| -> Result<Vec<(Id, Id, String)>> {
            Ok(store.conn.prepare(
                "SELECT source_id,file_id,relative FROM source_files ORDER BY source_id,file_id",
            )?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<Result<_, _>>()?)
        };
        let original_links = links(&store)?;
        let mut invalid = draft.clone();
        invalid.files[0].chapters.push(Chapter {
            title: "Invalid timestamp".into(),
            start: u64::MAX,
            end: None,
        });
        assert!(store.import(vec![invalid]).is_err());
        assert_eq!(serde_json::to_value(store.load()?.media)?, snapshot);
        assert_eq!(links(&store)?, original_links);
        let after = store.import(vec![draft.clone()])?;
        assert_eq!(after.media.len(), 2);
        assert_eq!(
            after.parts.iter().map(|part| part.id).collect::<Vec<_>>(),
            before.parts.iter().map(|part| part.id).collect::<Vec<_>>()
        );
        assert_eq!(after.session.position, 12_345);
        assert_eq!(after.progress[0].position, 12_345);
        let source = after
            .sources
            .iter()
            .find(|s| s.uri == draft.root_uri)
            .unwrap()
            .id;
        for file in &draft.files {
            let saved = after
                .media
                .iter()
                .find(|m| m.identity == file.identity)
                .unwrap();
            let relative: String = store.conn.query_row(
                "SELECT relative FROM source_files WHERE source_id=?1 AND file_id=?2",
                params![source, saved.id],
                |r| r.get(0),
            )?;
            assert_eq!(relative, file.relative);
            if !overlapping {
                assert_eq!(saved.relative, file.relative);
            }
        }
    }
    Ok(())
}

#[test]
fn document_rescan_refreshes_renamed_paths_without_losing_progress() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let uri = "content://provider/tree/root/document/opaque-recording";
    let mut draft = Draft {
        root_uri: "content://provider/tree/root/document/book".into(),
        title: "Book".into(),
        author: String::new(),
        files: vec![Media {
            uri: uri.into(),
            identity: crate::source::document_identity(uri).unwrap(),
            relative: "Old name.wav".into(),
            title: "Part".into(),
            ..Default::default()
        }],
        include: true,
    };
    let mut before = store.import(vec![draft.clone()])?;
    before.session.current = Some(Target::Book(before.parts[0].id));
    before.update_progress(12_345, false);
    store.save(&before.session, &before.progress)?;
    // A provider can keep the document ID while its display name or parent changes.
    draft.files[0].relative = "Disc 1/New name.wav".into();
    let after = store.import(vec![draft])?;
    assert_eq!(after.media[0].relative, "Disc 1/New name.wav");
    let relative: String = store.conn.query_row(
        "SELECT relative FROM source_files WHERE source_id=?1 AND file_id=?2",
        params![after.sources[0].id, after.media[0].id],
        |r| r.get(0),
    )?;
    assert_eq!(relative, after.media[0].relative);
    assert_eq!(after.parts[0].id, before.parts[0].id);
    assert_eq!(after.session.position, 12_345);
    assert_eq!(after.progress[0].position, 12_345);
    // A new document may reuse the old name and appear first in scan order.
    let mut moved = after.media[0].clone();
    moved.relative = "Moved.wav".into();
    let new_uri = "content://provider/tree/root/document/new-recording";
    let replacement = Media {
        uri: new_uri.into(),
        identity: crate::source::document_identity(new_uri).unwrap(),
        relative: "Disc 1/New name.wav".into(),
        ..Default::default()
    };
    let after = store.import(vec![Draft {
        root_uri: after.sources[0].uri.clone(),
        title: "Book".into(),
        author: String::new(),
        files: vec![replacement, moved],
        include: true,
    }])?;
    assert_eq!(after.media.len(), 2);
    assert_eq!(after.media[0].relative, "Moved.wav");
    assert_eq!(after.parts[0].id, before.parts[0].id);
    assert_eq!(after.progress[0].position, 12_345);
    Ok(())
}
