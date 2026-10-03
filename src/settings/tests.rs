use super::*;
#[test]
#[cfg(unix)]
fn saving_does_not_follow_a_stale_temporary_symlink() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("settings.json");
    let unrelated = dir.path().join("unrelated");
    fs::write(&unrelated, "keep me")?;
    let temp = path.with_extension(format!("json.tmp-{}", std::process::id()));
    std::os::unix::fs::symlink(&unrelated, &temp)?;
    Settings::default().save(&path)?;
    assert_eq!(fs::read_to_string(&unrelated)?, "keep me");
    assert!(temp.is_symlink());
    assert!(Settings::load(&path).1.is_none());
    Ok(())
}
#[test]
fn atomic_roundtrip_and_validation() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("config/settings.json");
    let s = Settings {
        window_width: 0,
        window_height: 9999,
        skip_silence: true,
        playback_rate: 1.75,
        text_scale: 1.23,
        android_ui_scale: 0.87,
        library_sort: "genre".into(),
        library_tab: "started".into(),
        ..Settings::default()
    };
    s.save(&path)?;
    let (s, warning, writable) = Settings::load(&path);
    assert!(warning.is_none() && writable);
    assert_eq!((s.window_width, s.window_height), (360, 4320));
    assert!(s.skip_silence);
    assert_eq!(s.playback_rate, 1.75);
    assert_eq!(s.text_scale, 1.23);
    assert_eq!(s.android_ui_scale, 0.87);
    assert_eq!(s.library_sort, "genre");
    assert_eq!(s.library_tab, "started");
    assert_eq!(fs::read_dir(path.parent().unwrap())?.count(), 1);
    Ok(())
}
#[test]
fn rate_defaults_for_existing_settings_and_rejects_invalid_values() -> Result<()> {
    let old: Settings = serde_json::from_str(r#"{"version":1,"theme":"light"}"#)?;
    assert_eq!(old.playback_rate, 1.0);
    for (input, expected) in [(f64::NAN, 1.0), (f64::INFINITY, 1.0), (0., 0.5), (4., 3.)] {
        let mut settings = Settings {
            playback_rate: input,
            ..Settings::default()
        };
        settings.validate()?;
        assert_eq!(settings.playback_rate, expected);
    }
    Ok(())
}
#[test]
fn legacy_text_scale_is_restored_and_removed_seek_settings_are_ignored() -> Result<()> {
    let mut old: Settings =
        serde_json::from_str(r#"{"version":1,"text_scale":2.0,"seek_back":45,"seek_forward":60}"#)?;
    old.validate()?;
    assert!(!old.skip_silence);
    assert_eq!(old.library_sort, "listening");
    assert_eq!(old.library_tab, "all");
    assert_eq!(old.text_scale, 2.0);
    assert_eq!(old.android_ui_scale, 1.0);
    assert!(serde_json::to_value(&old)?.get("seek_back").is_none());
    assert!(serde_json::to_value(&old)?.get("seek_forward").is_none());
    Ok(())
}
#[test]
fn display_scales_default_and_validate_independently() -> Result<()> {
    let old: Settings = serde_json::from_str(r#"{"version":1}"#)?;
    assert_eq!((old.text_scale, old.android_ui_scale), (1.0, 1.0));
    for (input, text, ui) in [
        (f32::NAN, 1.0, 1.0),
        (f32::INFINITY, 1.0, 1.0),
        (f32::NEG_INFINITY, 1.0, 1.0),
        (-1.0, 0.7, 0.7),
        (3.0, 2.05, 1.6),
        (1.234, 1.23, 1.23),
        (0.876, 0.88, 0.88),
    ] {
        let mut settings = Settings {
            text_scale: input,
            android_ui_scale: input,
            ..Settings::default()
        };
        settings.validate()?;
        assert_eq!((settings.text_scale, settings.android_ui_scale), (text, ui));
    }
    Ok(())
}
#[test]
fn completed_and_unknown_tabs_are_not_restored() -> Result<()> {
    for tab in ["completed", "unknown"] {
        let mut settings = Settings {
            library_tab: tab.into(),
            ..Settings::default()
        };
        settings.validate()?;
        assert_eq!(settings.library_tab, "all");
    }
    Ok(())
}
#[test]
fn corrupt_and_future_settings_preserve_original() -> Result<()> {
    let dir = tempfile::tempdir()?;
    let p = dir.path().join("settings.json");
    fs::write(&p, b"broken")?;
    let (_, warning, writable) = Settings::load(&p);
    assert!(warning.is_some() && writable);
    assert_eq!(fs::read(&p)?, b"broken");
    assert_eq!(fs::read_dir(dir.path())?.count(), 2);
    fs::write(&p, br#"{"version":999}"#)?;
    let (_, warning, writable) = Settings::load(&p);
    assert!(warning.is_some() && !writable);
    assert_eq!(fs::read(&p)?, br#"{"version":999}"#);
    Ok(())
}
