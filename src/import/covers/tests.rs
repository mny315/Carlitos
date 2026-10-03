use super::*;
#[test]
fn jpeg_covers_follow_exif_orientation_before_caching() -> Result<()> {
    use image::ImageEncoder;
    let dir = tempfile::tempdir()?;
    let original = dir.path().join("phone.jpg");
    let pixels = image::RgbImage::from_fn(32, 48, |x, _| {
        if x < 16 {
            image::Rgb([220, 20, 20])
        } else {
            image::Rgb([20, 20, 220])
        }
    });
    // Little-endian TIFF with a single SHORT Orientation=6 (90° clockwise).
    let exif = vec![
        b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
    ];
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95);
    encoder.set_exif_metadata(exif)?;
    encoder.encode_image(&pixels)?;
    std::fs::write(&original, bytes)?;
    let saved = custom_cover(&original, &dir.path().join("covers"))?;
    let decoded = load_cover_image(Path::new(&saved))?.to_rgb8();
    assert_eq!(decoded.dimensions(), (48, 32));
    let top = decoded.get_pixel(24, 4);
    let bottom = decoded.get_pixel(24, 28);
    assert!(top[0] > 180 && top[2] < 50, "red half must be on top");
    assert!(
        bottom[2] > 180 && bottom[0] < 50,
        "blue half must be on bottom"
    );
    assert_eq!(custom_cover(&original, &dir.path().join("covers"))?, saved);
    Ok(())
}

#[test]
fn cover_output_allocation_remains_bounded_when_reading_orientation() -> Result<()> {
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new(&mut bytes)
        .encode_image(&image::RgbImage::new(32, 48))?;
    let frame = bytes.windows(2).position(|w| w == [0xff, 0xc0]).unwrap();
    // Each dimension is valid, but the advertised RGB output exceeds 64 MiB.
    bytes[frame + 5..frame + 7].copy_from_slice(&5000u16.to_be_bytes());
    bytes[frame + 7..frame + 9].copy_from_slice(&5000u16.to_be_bytes());
    let error = decode_cover(&bytes).unwrap_err();
    assert!(matches!(
        error.downcast_ref::<image::ImageError>(),
        Some(image::ImageError::Limits(_))
    ));
    Ok(())
}

#[test]
fn managed_cover_paths_survive_a_different_launch_directory() -> Result<()> {
    let cwd = std::env::current_dir()?;
    let temp = tempfile::tempdir_in(&cwd)?;
    let original = temp.path().join("original.png");
    image::RgbImage::from_pixel(20, 30, image::Rgb([12, 24, 36])).save(&original)?;
    let relative = temp.path().strip_prefix(&cwd)?.join("covers");
    let saved = custom_cover(&original, &relative)?;
    assert!(
        Path::new(&saved).is_absolute(),
        "saved covers must not depend on the launch directory"
    );
    assert_eq!(load_cover_image(&Path::new("/").join(saved))?.width(), 20);
    Ok(())
}

#[test]
fn regenerating_a_cover_repairs_a_truncated_cache_file() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let original = temp.path().join("original.png");
    image::RgbImage::from_pixel(20, 30, image::Rgb([12, 24, 36])).save(&original)?;
    let cached = custom_cover(&original, &temp.path().join("covers"))?;
    let length = std::fs::metadata(&cached)?.len() as usize;
    for damaged in [b"partial PNG".to_vec(), vec![0; length]] {
        std::fs::write(&cached, damaged)?;
        // Also cover replacement with the same length: the worker's memo
        // must notice mtime changes, rather than trusting the old decode.
        std::fs::OpenOptions::new()
            .write(true)
            .open(&cached)?
            .set_modified(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000))?;
        assert_eq!(
            custom_cover(&original, &temp.path().join("covers"))?,
            cached
        );
        assert_eq!(image::ImageReader::open(&cached)?.decode()?.width(), 20);
    }
    Ok(())
}
#[test]
fn covers_reject_devices_and_directories_before_reading() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let paths = [
        #[cfg(unix)]
        Path::new("/dev/zero"),
        temp.path(),
    ];
    for path in paths {
        let error = custom_cover(path, &temp.path().join("covers")).unwrap_err();
        assert_eq!(
            error.to_string(),
            crate::i18n::tr("Обложка должна быть обычным файлом")
        );
    }
    assert!(!temp.path().join("covers").exists());
    Ok(())
}
#[test]
fn custom_covers_are_independent_validated_copies() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let source = temp.path().join("my cover.png");
    let directory = temp.path().join("library/covers");
    image::RgbImage::from_pixel(160, 240, image::Rgb([41, 74, 91])).save(&source)?;
    let saved = custom_cover(&source, &directory)?;
    assert_eq!(custom_cover(&source, &directory)?, saved);
    std::fs::remove_file(&source)?;
    assert!(Path::new(&saved).is_file());
    assert_eq!(
        image::ImageReader::open(&saved)?.into_dimensions()?,
        (160, 240)
    );
    std::fs::write(&source, b"not an image")?;
    assert!(custom_cover(&source, &directory).is_err());
    assert!(Path::new(&saved).is_file());
    Ok(())
}
