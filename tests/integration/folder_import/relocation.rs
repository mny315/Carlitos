use super::*;

#[test]
fn relocating_a_child_refreshes_its_links_in_an_overlapping_parent() -> anyhow::Result<()> {
    for outside in [false, true] {
        let temp = tempdir()?;
        let root = temp.path().join("collection");
        audio(&root, "book/1.wav")?;
        let mut store = Store::open(&temp.path().join("db"))?;
        let first = store.import(
            scan(
                vec![root.join("book")],
                ImportMode::Book,
                ScanControl::default(),
            )?
            .drafts,
        )?;
        let child = first.sources[0].id;
        let library = store
            .import(scan(vec![root.clone()], ImportMode::Book, ScanControl::default())?.drafts)?;
        let parent = library.sources.iter().find(|s| s.id != child).unwrap().id;
        let moved = if outside {
            temp.path().join("moved")
        } else {
            root.join("renamed")
        };
        std::fs::rename(root.join("book"), &moved)?;
        store.relocate(child, &moved)?;
        let new_root = temp.path().join("new-collection");
        std::fs::rename(&root, &new_root)?;
        let after = store.relocate(parent, &new_root)?;
        assert_eq!(after.parts[0].id, first.parts[0].id);
        assert!(local_path(&after.media[0].uri).unwrap().is_file());
        assert_eq!(after.source_book_counts()[&parent], usize::from(!outside));
    }
    Ok(())
}

#[test]
fn relocation_keeps_independent_hardlink_sources() -> anyhow::Result<()> {
    let temp = tempdir()?;
    let root = temp.path().join("original");
    let alias = temp.path().join("alias");
    audio(&root, "1.wav")?;
    std::fs::create_dir(&alias)?;
    std::fs::hard_link(root.join("1.wav"), alias.join("1.wav"))?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let library = store.import(
        scan(
            vec![root.clone(), alias.clone()],
            ImportMode::Book,
            ScanControl::default(),
        )?
        .drafts,
    )?;
    let original = library
        .sources
        .iter()
        .find(|s| local_path(&s.uri).as_ref() == Some(&root))
        .unwrap()
        .id;
    let linked = library
        .sources
        .iter()
        .find(|s| local_path(&s.uri).as_ref() == Some(&alias))
        .unwrap()
        .id;
    let moved = temp.path().join("moved");
    std::fs::rename(&root, &moved)?;
    store.relocate(original, &moved)?;
    // A later relocation of the alias must still validate its linked file.
    let alias_moved = temp.path().join("alias-moved");
    std::fs::rename(&alias, &alias_moved)?;
    std::fs::remove_file(alias_moved.join("1.wav"))?;
    assert!(store.relocate(linked, &alias_moved).is_err());
    Ok(())
}

#[test]
fn relocating_a_hardlink_alias_keeps_the_media_path_relative_to_its_source() -> anyhow::Result<()> {
    let temp = tempdir()?;
    let root = temp.path().join("original");
    let alias = temp.path().join("alias");
    audio(&root, "1.wav")?;
    std::fs::create_dir(&alias)?;
    std::fs::hard_link(root.join("1.wav"), alias.join("1.wav"))?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let library = store.import(
        scan(
            vec![root, alias.clone()],
            ImportMode::Book,
            ScanControl::default(),
        )?
        .drafts,
    )?;
    let linked = library
        .sources
        .iter()
        .find(|s| local_path(&s.uri).as_ref() == Some(&alias))
        .unwrap()
        .id;
    let moved = temp.path().join("alias-moved");
    std::fs::rename(&alias, &moved)?;
    let after = store.relocate(linked, &moved)?;
    let file = &after.media[0];
    let source = after
        .sources
        .iter()
        .find(|s| s.id == file.source_id)
        .unwrap();
    assert_eq!(
        local_path(&source.uri).unwrap().join(&file.relative),
        local_path(&file.uri).unwrap(),
    );
    assert_eq!(file.id, library.media[0].id);
    assert_eq!(after.parts[0].id, library.parts[0].id);
    let after = store.remove_source(linked)?;
    assert_eq!(after.media[0].uri, library.media[0].uri);
    assert_eq!(after.parts[0].id, library.parts[0].id);
    Ok(())
}

#[test]
fn relocating_a_parent_updates_nested_sources_and_validates_all_their_files() -> anyhow::Result<()>
{
    let temp = tempdir()?;
    let old = temp.path().join("old");
    audio(&old, "nested/1.wav")?;
    audio(&old, "nested/2.wav")?;
    let mut store = Store::open(&temp.path().join("db"))?;
    let inner = scan(
        vec![old.join("nested")],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    let before = store.import(inner.drafts)?;
    let mut outer = scan(vec![old.clone()], ImportMode::Book, ScanControl::default())?;
    outer.drafts[0].files.pop();
    let library = store.import(outer.drafts)?;
    let source = library
        .sources
        .iter()
        .find(|s| local_path(&s.uri).as_deref() == Some(old.as_path()))
        .unwrap()
        .id;
    assert_eq!(library.source_book_counts()[&source], 1);
    let new = temp.path().join("new");
    std::fs::rename(&old, &new)?;
    let missing = new.join("nested/2.wav");
    let saved = std::fs::read(&missing)?;
    std::fs::remove_file(&missing)?;
    assert!(
        store.relocate(source, &new).is_err(),
        "must validate files linked only through the nested source"
    );
    assert!(
        store
            .load()?
            .sources
            .iter()
            .all(|s| local_path(&s.uri).unwrap().starts_with(&old))
    );
    std::fs::write(&missing, saved)?;
    let after = store.relocate(source, &new)?;
    assert_eq!(
        after.parts.iter().map(|p| p.id).collect::<Vec<_>>(),
        before.parts.iter().map(|p| p.id).collect::<Vec<_>>()
    );
    for source in &after.sources {
        let root = local_path(&source.uri).unwrap();
        assert!(root.starts_with(&new) && root.is_dir());
    }
    for media in &after.media {
        let path = local_path(&media.uri).unwrap();
        assert!(path.starts_with(&new) && path.is_file());
    }
    Ok(())
}
