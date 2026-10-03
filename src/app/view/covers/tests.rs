use super::*;

fn receive_cover(receiver: &async_channel::Receiver<CoverResult>) -> anyhow::Result<CoverResult> {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        if let Ok(result) = receiver.try_recv() {
            return Ok(result);
        }
        anyhow::ensure!(std::time::Instant::now() < until, "cover request was lost");
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

#[test]
#[cfg(unix)]
fn a_named_pipe_cover_cannot_block_the_decoder_or_shutdown() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("cover.png");
    anyhow::ensure!(
        std::process::Command::new("mkfifo")
            .arg(&path)
            .status()?
            .success()
    );
    let (covers, receiver) = Covers::new();
    covers.image(path.to_str());
    let result = receive_cover(&receiver);
    // Unblock the old implementation even when the assertion fails. Once
    // open returns, format detection rejects this non-seekable descriptor.
    let guard = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)?;
    drop(covers);
    drop(guard);
    assert!(result?.pixels.is_none());
    Ok(())
}

#[test]
fn cover_requests_survive_a_full_background_queue() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (covers, receiver) = Covers::new();
    let mut paths = Vec::new();
    for i in 0..80 {
        let path = dir.path().join(format!("{i}.png"));
        image::RgbImage::from_pixel(8, 8, image::Rgb([12, 34, 56])).save(&path)?;
        paths.push(path.to_string_lossy().into_owned());
    }
    for path in &paths {
        covers.image(Some(path));
    }
    for _ in &paths {
        covers.accept(receive_cover(&receiver)?);
    }
    for path in &paths {
        assert!(covers.image(Some(path)).size().width > 0);
    }
    Ok(())
}

#[test]
fn pending_covers_do_not_supply_a_temporary_neutral_palette() -> anyhow::Result<()> {
    for valid in [true, false] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("cover.png");
        if valid {
            image::RgbImage::from_pixel(8, 8, image::Rgb([30, 80, 180])).save(&path)?;
        }
        let (covers, receiver) = Covers::new();
        covers.image(path.to_str());
        let result = receive_cover(&receiver)?;
        assert!(covers.palette(path.to_str()).is_none());
        covers.accept(result);
        let palette = covers.palette(path.to_str()).unwrap();
        assert_eq!(palette == CoverPalette::default(), !valid);
        assert_eq!(covers.palette(None), Some(CoverPalette::default()));
    }
    Ok(())
}

#[test]
fn current_palette_survives_scrolling_beyond_the_image_cache() -> anyhow::Result<()> {
    let dir = tempfile::tempdir()?;
    let (covers, receiver) = Covers::new();
    let current = dir.path().join("0.png").to_string_lossy().into_owned();
    for i in 0..140 {
        let path = dir.path().join(format!("{i}.png"));
        image::RgbImage::from_pixel(8, 8, image::Rgb([30, 80, 180])).save(&path)?;
        covers.image(path.to_str());
        covers.accept(receive_cover(&receiver)?);
        // The player asks for its current cover on each UI refresh.
        assert!(covers.image(Some(&current)).size().width > 0);
    }
    assert_eq!(covers.images.borrow().len(), 128);
    let palette = covers.palette(Some(&current)).unwrap();
    assert!(palette.dark.blue() > palette.dark.red());
    Ok(())
}

#[test]
fn repaired_covers_replace_failed_and_inflight_decode_results() -> anyhow::Result<()> {
    for inflight in [false, true] {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("cover.png");
        let path = path.to_string_lossy().into_owned();
        std::fs::write(&path, b"broken PNG")?;
        let (covers, receiver) = Covers::new();
        covers.image(Some(&path));
        let failed = receive_cover(&receiver)?;
        assert!(failed.pixels.is_none());
        let mut failed = Some(failed);
        if !inflight {
            covers.accept(failed.take().unwrap());
        }
        image::RgbImage::from_pixel(8, 8, image::Rgb([12, 34, 56])).save(&path)?;
        covers.retry(&path);
        covers.image(Some(&path));
        if let Some(failed) = failed {
            covers.accept(failed);
        }
        covers.accept(receive_cover(&receiver)?);
        assert!(covers.image(Some(&path)).size().width > 0);
        assert_ne!(
            covers.palette(Some(&path)).unwrap(),
            CoverPalette::default()
        );
    }
    Ok(())
}
