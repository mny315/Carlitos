use super::*;
use crate::import::reader::DocumentReader;
use lofty::{
    config::WriteOptions,
    picture::Picture,
    tag::{Tag, TagType},
};
use std::{io::Cursor, path::Path, time::Duration};

fn fixtures(root: &Path) -> Result<()> {
    let mut image = Cursor::new(Vec::new());
    image::RgbImage::from_pixel(24, 32, image::Rgb([20, 70, 120]))
        .write_to(&mut image, image::ImageFormat::Png)?;
    for (disc, track, name) in [(1, 1, "10.mp3"), (1, 2, "2.mp3"), (2, 1, "1.mp3")] {
        let folder = root.join(format!("Автор/Книга/CD {disc}"));
        std::fs::create_dir_all(&folder)?;
        let path = folder.join(name);
        std::fs::write(
            &path,
            include_bytes!("../../../tests/fixtures/import-stereo.mp3"),
        )?;
        let mut tag = Tag::new(TagType::Id3v2);
        tag.set_title(format!("Глава {disc}.{track}"));
        tag.set_album("Книга с тегами".into());
        tag.set_artist("Автор Кириллица".into());
        tag.set_track(track);
        tag.set_disk(disc);
        tag.set_genre("Аудиокнига".into());
        tag.insert_text(lofty::tag::ItemKey::RecordingDate, "2024".into());
        tag.push_picture(Picture::from_reader(&mut Cursor::new(image.get_ref()))?);
        tag.save_to_path(path, WriteOptions::default())?;
    }
    let formats = root.join("Форматы");
    std::fs::create_dir_all(&formats)?;
    for (name, bytes) in [
        (
            "Моно.mp3",
            &include_bytes!("../../../tests/fixtures/import-mono.mp3")[..],
        ),
        (
            "AAC.aac",
            &include_bytes!("../../../tests/fixtures/import.aac")[..],
        ),
        (
            "FLAC.flac",
            &include_bytes!("../../../tests/fixtures/import.flac")[..],
        ),
        (
            "M4A.m4a",
            &include_bytes!("../../../tests/fixtures/import.m4b")[..],
        ),
        (
            "HE-AAC v1.m4b",
            &include_bytes!("../../../tests/fixtures/import-he-aac-v1.m4b")[..],
        ),
        (
            "HE-AAC v2.m4b",
            &include_bytes!("../../../tests/fixtures/import-he-aac-v2.m4b")[..],
        ),
        (
            "Nero.m4b",
            &include_bytes!("../../../tests/fixtures/import.m4b")[..],
        ),
        (
            "QuickTime.m4b",
            &include_bytes!("../../../tests/fixtures/import.m4b")[..],
        ),
    ] {
        let path = formats.join(name);
        std::fs::write(&path, bytes)?;
        if name.ends_with(".m4b") {
            let mut tag = mp4ameta::Tag::read_from_path(&path)?;
            tag.set_title(name);
            tag.set_artist("Автор глав");
            let chapters = if name == "Nero.m4b" {
                tag.chapter_list_mut()
            } else {
                tag.chapter_track_mut()
            };
            chapters.extend([
                mp4ameta::Chapter::new(Duration::ZERO, "Начало"),
                mp4ameta::Chapter::new(Duration::from_millis(500), "Продолжение"),
            ]);
            tag.write_to_path(path)?;
        }
    }
    let natural = root.join("Естественный порядок");
    std::fs::create_dir_all(&natural)?;
    for name in ["1.wav", "2.wav", "10.wav"] {
        let mut bytes = Vec::new();
        bytes.extend(b"RIFF");
        bytes.extend(16036u32.to_le_bytes());
        bytes.extend(b"WAVEfmt ");
        bytes.extend(16u32.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(1u16.to_le_bytes());
        bytes.extend(8000u32.to_le_bytes());
        bytes.extend(16000u32.to_le_bytes());
        bytes.extend(2u16.to_le_bytes());
        bytes.extend(16u16.to_le_bytes());
        bytes.extend(b"data");
        bytes.extend(16000u32.to_le_bytes());
        for n in 0..8000 {
            bytes.extend(
                (((n as f64 * 440.0 * std::f64::consts::TAU / 8000.0).sin() * 4000.0) as i16)
                    .to_le_bytes(),
            );
        }
        std::fs::write(natural.join(name), bytes)?;
    }
    std::fs::write(natural.join("cover.png"), image.into_inner())?;
    std::fs::write(formats.join("Повреждённый.mp3"), b"not an audio file")?;
    Ok(())
}

#[test]
fn portable_tags_and_both_chapter_types_read_from_document_slices() -> Result<()> {
    let temp = tempfile::tempdir()?;
    fixtures(temp.path())?;
    for (relative, mp3) in [
        ("Автор/Книга/CD 1/10.mp3", true),
        ("Форматы/Nero.m4b", false),
        ("Форматы/QuickTime.m4b", false),
    ] {
        let bytes = std::fs::read(temp.path().join(relative))?;
        let mut container = b"outside prefix".to_vec();
        let offset = container.len() as u64;
        container.extend(&bytes);
        container.extend(b"outside suffix");
        let mut reader =
            DocumentReader::new(Cursor::new(container), offset, Some(bytes.len() as u64))?;
        let mut media = Media {
            duration: Some(1000),
            ..Default::default()
        };
        if mp3 {
            // Nix/CI need no writable user cache; test the same encoder in our own directory.
            read_with_cover(&mut reader, &mut media, |bytes| {
                crate::import::covers::save_cover(bytes, relative, temp.path())
            });
        } else {
            read(&mut reader, &mut media);
        }
        if mp3 {
            assert_eq!(media.title, "Глава 1.1");
            assert_eq!(media.album, "Книга с тегами");
            assert_eq!(media.artist, "Автор Кириллица");
            assert_eq!(
                (media.track, media.disc, media.year),
                (Some(1), Some(1), Some(2024))
            );
            assert_eq!(media.genre, "Аудиокнига");
            assert!(media.cover.as_ref().is_some_and(|p| Path::new(p).is_file()));
            let tags = read_sort_tags(&mut reader)?;
            assert_eq!(tags.year, Some(2024));
            assert_eq!(tags.genre, "Аудиокнига");
        } else {
            assert_eq!(media.artist, "Автор глав");
            assert_eq!(media.chapters.len(), 2, "{relative}");
            assert_eq!(media.chapters[0].title, "Начало");
            assert_eq!(media.chapters[0].end, Some(500));
            assert_eq!(media.chapters[1].start, 500);
            assert_eq!(media.chapters[1].end, Some(1000));
        }
    }
    Ok(())
}

#[test]
#[ignore = "generate synthetic device fixtures; set CARLITOS_ANDROID_FIXTURES"]
fn prepare_android_import_fixtures() -> Result<()> {
    fixtures(Path::new(&std::env::var("CARLITOS_ANDROID_FIXTURES")?))
}
