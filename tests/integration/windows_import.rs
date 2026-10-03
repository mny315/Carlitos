#![cfg(windows)]
use carlitos::{
    import::{ImportMode, ScanControl, scan},
    library::local_path,
    storage::Store,
};

#[test]
fn files_without_decodable_audio_are_reported_without_creating_books() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    std::fs::write(temp.path().join("empty.mp3"), [])?;
    std::fs::write(temp.path().join("broken.wav"), b"not audio")?;
    // A valid PCM container with no samples must not count as decoded audio.
    let mut wav = b"RIFF".to_vec();
    wav.extend(36u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(0u32.to_le_bytes());
    std::fs::write(temp.path().join("header-only.wav"), wav)?;
    let result = scan(
        vec![temp.path().into()],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    assert!(result.drafts.is_empty());
    for name in ["empty.mp3", "broken.wav", "header-only.wav"] {
        assert!(
            result.issues.iter().any(|issue| issue.contains(name)),
            "{name}: {:?}",
            result.issues
        );
    }
    Ok(())
}

#[test]
fn mp3_books_with_cyrillic_paths_tags_and_covers() -> anyhow::Result<()> {
    use lofty::{
        config::WriteOptions,
        picture::Picture,
        prelude::*,
        tag::{Tag, TagType},
    };
    let temp = tempfile::tempdir()?;
    let mut cover = std::io::Cursor::new(Vec::new());
    image::RgbImage::from_pixel(16, 16, image::Rgb([20, 40, 80]))
        .write_to(&mut cover, image::ImageFormat::Png)?;
    for (name, bytes) in [
        (
            "Моно книга",
            &include_bytes!("../fixtures/import-mono.mp3")[..],
        ),
        (
            "Стерео книга",
            &include_bytes!("../fixtures/import-stereo.mp3")[..],
        ),
    ] {
        let folder = temp.path().join("Автор с пробелами").join(name);
        std::fs::create_dir_all(&folder)?;
        for track in [2, 1] {
            let path = folder.join(format!("Глава {track}.mp3"));
            std::fs::write(&path, bytes)?;
            let mut tag = Tag::new(TagType::Id3v2);
            tag.set_album(name.into());
            tag.set_artist("Автор".into());
            tag.set_title(format!("Глава {track}"));
            tag.set_track(track);
            tag.push_picture(Picture::from_reader(&mut std::io::Cursor::new(
                cover.get_ref(),
            ))?);
            tag.save_to_path(&path, WriteOptions::default())?;
        }
    }
    let result = scan(
        vec![temp.path().into()],
        ImportMode::Books,
        ScanControl::default(),
    )?;
    assert!(result.issues.is_empty(), "{:?}", result.issues);
    assert_eq!(result.drafts.len(), 2);
    for book in &result.drafts {
        assert_eq!(book.author, "Автор");
        assert_eq!(book.files.len(), 2);
        for (index, file) in book.files.iter().enumerate() {
            assert_eq!(file.track, Some(index as u32 + 1));
            assert!(file.duration.is_some_and(|d| (900..=1200).contains(&d)));
            assert!(
                file.cover
                    .as_ref()
                    .is_some_and(|p| std::path::Path::new(p).is_file())
            );
        }
    }
    let mut store = Store::open(&temp.path().join("library.sqlite3"))?;
    let library = store.import(result.drafts)?;
    assert_eq!(library.books.len(), 2);
    assert_eq!(library.parts.len(), 4);
    assert_eq!(store.load()?.media.len(), 4);
    Ok(())
}

#[test]
fn aac_flac_and_m4b_chapters_import() -> anyhow::Result<()> {
    use std::time::Duration;
    let temp = tempfile::tempdir()?;
    for (name, bytes) in [
        ("Книга.aac", &include_bytes!("../fixtures/import.aac")[..]),
        ("Книга.flac", &include_bytes!("../fixtures/import.flac")[..]),
        (
            "Список глав.m4b",
            &include_bytes!("../fixtures/import.m4b")[..],
        ),
        (
            "Дорожка глав.m4b",
            &include_bytes!("../fixtures/import.m4b")[..],
        ),
        (
            "HE-AAC v1.m4b",
            &include_bytes!("../fixtures/import-he-aac-v1.m4b")[..],
        ),
        (
            "HE-AAC v2.m4b",
            &include_bytes!("../fixtures/import-he-aac-v2.m4b")[..],
        ),
    ] {
        let path = temp.path().join(name);
        std::fs::write(&path, bytes)?;
        if name.ends_with(".m4b") {
            let mut tag = mp4ameta::Tag::read_from_path(&path)?;
            tag.set_title(name);
            tag.set_album("Книга с главами");
            tag.set_artist("Автор");
            let chapters = if name.starts_with("Список") {
                tag.chapter_list_mut()
            } else {
                tag.chapter_track_mut()
            };
            chapters.extend([
                mp4ameta::Chapter::new(Duration::ZERO, "Начало"),
                mp4ameta::Chapter::new(Duration::from_millis(500), "Продолжение"),
            ]);
            tag.write_to_path(&path)?;
        }
    }
    let result = scan(
        vec![temp.path().into()],
        ImportMode::Book,
        ScanControl::default(),
    )?;
    assert!(result.issues.is_empty(), "{:?}", result.issues);
    assert_eq!(result.drafts.len(), 1);
    assert_eq!(result.drafts[0].files.len(), 6);
    for media in &result.drafts[0].files {
        if !media.uri.ends_with(".aac") || media.duration.is_some() {
            assert!(
                media.duration.is_some_and(|d| (900..=1300).contains(&d)),
                "{}: {:?}",
                media.uri,
                media.duration
            );
        }
        if media.uri.ends_with(".m4b") {
            assert_eq!(media.artist, "Автор");
            assert_eq!(media.chapters.len(), 2, "{}", media.uri);
            assert_eq!(media.chapters[0].title, "Начало");
            assert_eq!(media.chapters[0].start, 0);
            assert_eq!(media.chapters[0].end, Some(500));
            assert_eq!(media.chapters[1].start, 500);
            assert_eq!(media.chapters[1].end, media.duration);
        }
    }
    let mut store = Store::open(&temp.path().join("library.sqlite3"))?;
    let library = store.import(result.drafts)?;
    assert_eq!(library.media.len(), 6);
    assert_eq!(library.parts.len(), 6);
    for media in library.media.iter().filter(|m| m.uri.ends_with(".m4b")) {
        assert_eq!(media.chapters.len(), 2);
        assert_eq!(media.chapters[1].title, "Продолжение");
        assert_eq!(media.chapters[1].start, 500);
    }
    Ok(())
}

#[test]
#[ignore = "set CARLITOS_IMPORT_DIR to a real audiobook collection (read-only input)"]
fn real_audiobook_collection() -> anyhow::Result<()> {
    let root = std::path::PathBuf::from(std::env::var("CARLITOS_IMPORT_DIR")?);
    let started = std::time::Instant::now();
    let result = scan(vec![root], ImportMode::Books, ScanControl::default())?;
    for issue in &result.issues {
        eprintln!("{issue}");
    }
    let files: usize = result.drafts.iter().map(|draft| draft.files.len()).sum();
    println!(
        "Scanned {} books, {files} files, {} issues in {:?}",
        result.drafts.len(),
        result.issues.len(),
        started.elapsed()
    );
    for draft in &result.drafts {
        println!("{}: {} parts", draft.title, draft.files.len());
        for media in &draft.files {
            assert!(media.duration.is_some_and(|d| d > 0), "{}", media.uri);
            assert!(local_path(&media.uri).is_some_and(|p| p.is_file()));
        }
    }
    assert!(
        result.issues.is_empty(),
        "Some inputs could not be imported"
    );
    assert!(files > 0, "No audio files found");
    let temp = tempfile::tempdir()?;
    let mut store = Store::open(&temp.path().join("library.sqlite3"))?;
    let library = store.import(result.drafts.clone())?;
    assert_eq!(library.books.len(), result.drafts.len());
    assert_eq!(library.media.len(), files);
    let repeated = store.import(result.drafts)?;
    assert_eq!(repeated.books.len(), library.books.len());
    assert_eq!(repeated.parts.len(), library.parts.len());
    Ok(())
}
