use super::*;

#[test]
fn import_reads_both_mp4_chapter_formats() -> anyhow::Result<()> {
    use std::time::Duration;

    let temp = tempdir()?;
    for chapter_track in [false, true] {
        let path = temp.path().join(if chapter_track {
            "QuickTime.m4b"
        } else {
            "Nero.m4b"
        });
        std::fs::write(&path, include_bytes!("../../fixtures/import.m4b"))?;
        let mut tag = mp4ameta::Tag::read_from_path(&path)?;
        let chapters = if chapter_track {
            tag.chapter_track_mut()
        } else {
            tag.chapter_list_mut()
        };
        chapters.extend([
            mp4ameta::Chapter::new(Duration::ZERO, "Начало"),
            mp4ameta::Chapter::new(Duration::from_millis(500), "Продолжение"),
        ]);
        tag.write_to_path(&path)?;
        let result = scan(vec![path], ImportMode::Book, ScanControl::default())?;
        assert!(result.issues.is_empty(), "{:?}", result.issues);
        let media = &result.drafts[0].files[0];
        assert_eq!(media.chapters.len(), 2, "chapter track: {chapter_track}");
        assert_eq!(media.chapters[0].title, "Начало");
        assert_eq!(media.chapters[0].start, 0);
        assert_eq!(media.chapters[0].end, Some(500));
        assert_eq!(media.chapters[1].title, "Продолжение");
        assert_eq!(media.chapters[1].start, 500);
        assert_eq!(media.chapters[1].end, media.duration);
    }
    Ok(())
}

#[test]
fn unavailable_inputs_do_not_discard_other_scanned_files() -> anyhow::Result<()> {
    let temp = tempdir()?;
    audio(temp.path(), "book/1.wav")?;
    let result = scan(
        vec![
            temp.path().join("missing/book/1.wav"),
            temp.path().join("book/1.wav"),
        ],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    assert_eq!(result.drafts.len(), 1);
    assert_eq!(result.issues.len(), 1);
    assert!(result.issues[0].contains("missing"));
    Ok(())
}

#[test]
#[cfg(unix)]
fn individual_files_through_a_directory_alias_share_the_canonical_source() -> anyhow::Result<()> {
    let temp = tempdir()?;
    let root = temp.path().join("book");
    audio(&root, "1.wav")?;
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias)?;
    let mut store = Store::open(&temp.path().join("db"))?;
    for path in [root.join("1.wav"), alias.join("1.wav")] {
        let result = scan(vec![path], ImportMode::Book, ScanControl::default())?;
        assert!(result.issues.is_empty());
        let library = store.import(result.drafts)?;
        assert_eq!(library.sources.len(), 1);
        assert_eq!(library.parts.len(), 1);
    }
    Ok(())
}

#[test]
#[cfg(unix)]
fn directory_aliases_share_one_source_and_empty_folders_report_a_result() -> anyhow::Result<()> {
    let temp = tempdir()?;
    let root = temp.path().join("book");
    audio(&root, "1.wav")?;
    std::fs::create_dir(root.join("empty"))?;
    let alias = temp.path().join("alias");
    std::os::unix::fs::symlink(&root, &alias)?;
    let mut store = Store::open(&temp.path().join("db"))?;
    for folder in [&root, &alias, &root.join("empty/..")] {
        let result = scan(
            vec![folder.clone()],
            ImportMode::Books,
            ScanControl::default(),
        )?;
        let library = store.import(result.drafts)?;
        assert_eq!(library.sources.len(), 1);
        assert_eq!(library.books.len(), 1);
        assert_eq!(library.parts.len(), 1);
    }
    let result = scan(
        vec![root.join("empty")],
        ImportMode::Books,
        ScanControl::default(),
    )?;
    assert!(result.drafts.is_empty());
    assert_eq!(result.issues.len(), 1);
    Ok(())
}

#[test]
fn collection_finds_nested_books_and_keeps_discs_together() -> anyhow::Result<()> {
    let temp = tempdir()?;
    let root = temp.path();
    for file in [
        "Автор/Книга 2/10.wav",
        "Автор/Книга 2/2.wav",
        "Автор/Книга 2/1.wav",
        "Автор/Книга 10/CD 1/1.wav",
        "Автор/Книга 10/CD 2/1.wav",
        "Другая книга/Диск 1/1.wav",
        "Другая книга/Диск 2/1.wav",
        "Отдельная запись.wav",
    ] {
        audio(root, file)?;
    }
    std::fs::create_dir(root.join("Обложки"))?;
    std::fs::write(root.join("Обложки/cover.jpg"), b"not audio")?;
    std::fs::write(root.join("Автор/Книга 2/broken.mp3"), b"broken")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(root, root.join("loop"))?;
    let result = scan(vec![root.into()], ImportMode::Books, ScanControl::default())?;
    assert_eq!(result.drafts.len(), 4);
    assert!(result.drafts.iter().all(|d| d.include));
    assert_eq!(
        result.drafts.iter().map(|d| d.files.len()).sum::<usize>(),
        8
    );
    for (folder, expected) in [
        ("Автор/Книга 2", vec!["1.wav", "2.wav", "10.wav"]),
        ("Автор/Книга 10", vec!["CD 1/1.wav", "CD 2/1.wav"]),
        ("Другая книга", vec!["Диск 1/1.wav", "Диск 2/1.wav"]),
    ] {
        let draft = result
            .drafts
            .iter()
            .find(|d| local_path(&d.root_uri).unwrap() == root.join(folder))
            .unwrap();
        assert_eq!(
            draft
                .files
                .iter()
                .map(|f| f.relative.replace('\\', "/"))
                .collect::<Vec<_>>(),
            expected
        );
        // Importing a single book via the default action gives the same group.
        let single = scan(
            vec![root.join(folder)],
            ImportMode::Books,
            ScanControl::default(),
        )?;
        assert_eq!(single.drafts.len(), 1);
        assert_eq!(single.drafts[0].root_uri, draft.root_uri);
        assert_eq!(single.drafts[0].files.len(), draft.files.len());
    }
    assert!(result.issues.iter().any(|s| s.contains("broken.mp3")));
    #[cfg(unix)]
    assert!(result.issues.iter().any(|s| s.contains("loop")));
    Ok(())
}

#[test]
fn import_reads_year_and_genre_from_audio_tags() -> anyhow::Result<()> {
    let temp = tempdir()?;
    audio(temp.path(), "tagged.wav")?;
    let file = temp.path().join("tagged.wav");
    let mut wav = std::fs::read(&file)?;
    let mut info = b"INFO".to_vec();
    for (id, value) in [(b"ICRD", "1997-01-01"), (b"IGNR", "Science fiction")] {
        let mut bytes = value.as_bytes().to_vec();
        bytes.push(0);
        info.extend(id);
        info.extend((bytes.len() as u32).to_le_bytes());
        info.extend(&bytes);
        if bytes.len() % 2 != 0 {
            info.push(0);
        }
    }
    wav.extend(b"LIST");
    wav.extend((info.len() as u32).to_le_bytes());
    wav.extend(info);
    let size = (wav.len() as u32 - 8).to_le_bytes();
    wav[4..8].copy_from_slice(&size);
    std::fs::write(&file, wav)?;
    let result = scan(vec![file.clone()], ImportMode::Book, ScanControl::default())?;
    let media = &result.drafts[0].files[0];
    assert_eq!(media.year, Some(1997));
    assert_eq!(media.genre, "Science fiction");
    assert!(media.sort_tags_read);
    let tags =
        carlitos::import::read_sort_tags(&file_uri(&file)?, &carlitos::import::discoverer()?)?;
    assert_eq!(tags.year, media.year);
    assert_eq!(tags.genre, media.genre);
    Ok(())
}
