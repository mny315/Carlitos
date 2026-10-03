use super::*;
use std::{io::Cursor, time::Duration};

#[test]
fn chapter_tracks_can_change_the_number_of_samples_per_chunk() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("chunks.m4b");
    std::fs::write(&path, include_bytes!("../../../tests/fixtures/import.m4b"))?;
    let mut tag = mp4ameta::Tag::read_from_path(&path)?;
    tag.chapter_track_mut().extend([
        mp4ameta::Chapter::new(Duration::ZERO, "One"),
        mp4ameta::Chapter::new(Duration::from_millis(300), "Two"),
        mp4ameta::Chapter::new(Duration::from_millis(600), "Three"),
    ]);
    tag.write_to_path(&path)?;
    let mut bytes = std::fs::read(&path)?;
    let co64 = bytes.windows(4).rposition(|w| w == b"co64").unwrap() - 4;
    let stsz = bytes.windows(4).rposition(|w| w == b"stsz").unwrap() - 4;
    let first_offset = u64::from_be_bytes(bytes[co64 + 16..co64 + 24].try_into()?) as u32;
    let first_size = u32::from_be_bytes(bytes[stsz + 20..stsz + 24].try_into()?);
    let moov = bytes.windows(4).position(|w| w == b"moov").unwrap() - 4;
    assert!((first_offset as usize) < moov, "samples precede metadata");
    // Split the existing contiguous samples into chunks of one and two.
    // Sample offsets stay valid because only the final moov atom grows.
    let offsets = [0, 2, 0, first_offset, 0, first_offset + first_size];
    bytes = replace_atom(&bytes, co64, b"co64", &offsets);
    let stsc = bytes.windows(4).rposition(|w| w == b"stsc").unwrap() - 4;
    bytes = replace_atom(&bytes, stsc, b"stsc", &[0, 2, 1, 1, 1, 2, 2, 1]);
    let chapters = read(&mut Cursor::new(&bytes), Some(1000));
    assert_eq!(
        chapters
            .iter()
            .map(|c| (c.title.as_str(), c.start))
            .collect::<Vec<_>>(),
        [("One", 0), ("Two", 300), ("Three", 600)]
    );
    for (first, next) in [(0, 2), (2, 2), (1, 0), (1, 1), (1, 3)] {
        let malformed = replace_atom(&bytes, stsc, b"stsc", &[0, 2, first, 1, 1, next, 2, 1]);
        assert!(mp4ameta::Tag::read_from(&mut Cursor::new(malformed)).is_err());
    }
    Ok(())
}

fn replace_atom(bytes: &[u8], target: usize, kind: &[u8; 4], fields: &[u32]) -> Vec<u8> {
    let mut result = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let size = u32::from_be_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
        let atom_kind = &bytes[offset + 4..offset + 8];
        let body = if offset == target {
            assert_eq!(atom_kind, kind);
            fields.iter().flat_map(|v| v.to_be_bytes()).collect()
        } else if offset < target && target < offset + size {
            replace_atom(
                &bytes[offset + 8..offset + size],
                target - offset - 8,
                kind,
                fields,
            )
        } else {
            bytes[offset + 8..offset + size].to_vec()
        };
        result.extend_from_slice(&((8 + body.len()) as u32).to_be_bytes());
        result.extend_from_slice(atom_kind);
        result.extend(body);
        offset += size;
    }
    result
}

#[test]
fn malformed_mp4_timescales_do_not_abort_metadata_workers() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("chapters.m4b");
    std::fs::write(&path, include_bytes!("../../../tests/fixtures/import.m4b"))?;
    let mut tag = mp4ameta::Tag::read_from_path(&path)?;
    tag.chapter_track_mut()
        .push(mp4ameta::Chapter::new(Duration::ZERO, "Chapter"));
    tag.write_to_path(&path)?;
    let original = std::fs::read(path)?;
    // The movie and chapter-track headers have independent timescales.
    for atom in [b"mvhd", b"mdhd"] {
        let mut bytes = original.clone();
        let offset = bytes.windows(4).rposition(|w| w == atom).unwrap();
        assert_eq!(bytes[offset + 4], 0, "fixture uses a version 0 header");
        bytes[offset + 16..offset + 20].fill(0);
        assert!(read(&mut Cursor::new(bytes), Some(1000)).is_empty());
    }
    assert_eq!(read(&mut Cursor::new(original), Some(1000)).len(), 1);
    Ok(())
}

#[test]
fn one_damaged_chapter_format_does_not_hide_the_other() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("both.m4b");
    std::fs::write(&path, include_bytes!("../../../tests/fixtures/import.m4b"))?;
    let mut tag = mp4ameta::Tag::read_from_path(&path)?;
    tag.chapter_list_mut()
        .push(mp4ameta::Chapter::new(Duration::ZERO, "List"));
    tag.chapter_track_mut()
        .push(mp4ameta::Chapter::new(Duration::ZERO, "Track"));
    tag.write_to_path(&path)?;
    let original = std::fs::read(path)?;
    for (damaged, expected) in [(b"mdhd", "List"), (b"chpl", "Track")] {
        let mut bytes = original.clone();
        let offset = bytes.windows(4).rposition(|w| w == damaged).unwrap();
        if damaged == b"mdhd" {
            bytes[offset + 16..offset + 20].fill(0);
        } else {
            bytes[offset + 4] = 255;
        }
        let chapters = read(&mut Cursor::new(bytes), Some(1000));
        assert_eq!(chapters.len(), 1, "damaged {damaged:?}");
        assert_eq!(chapters[0].title, expected);
        assert_eq!(chapters[0].end, Some(1000));
    }
    Ok(())
}
