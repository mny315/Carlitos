use std::{fs::Metadata, path::Path};

pub fn modified(metadata: &Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs().min(i64::MAX as u64) as i64)
}

#[cfg(unix)]
pub fn identity(path: &Path) -> anyhow::Result<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path)?;
    Ok(format!("{}:{}", meta.dev(), meta.ino()))
}

#[cfg(windows)]
pub fn identity(path: &Path) -> anyhow::Result<String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };
    let file = std::fs::File::open(path)?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    unsafe {
        GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info)?;
    }
    Ok(format!(
        "{}:{}",
        info.dwVolumeSerialNumber,
        (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)
    ))
}

#[cfg(windows)]
pub fn data_dir() -> std::path::PathBuf {
    use std::{ffi::OsString, os::windows::ffi::OsStringExt, path::PathBuf};
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{FOLDERID_LocalAppData, KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };

    let exe = std::env::current_exe().expect("executable path");
    let directory = exe.parent().expect("executable directory");
    // Only the installer ships this marker. Copying just the EXE remains portable,
    // and launching an installed copy directly uses the same data as its shortcuts.
    if !directory.join("carlitos-installed").is_file() {
        return directory.join("Carlitos-data");
    }
    unsafe {
        let path = SHGetKnownFolderPath(&FOLDERID_LocalAppData, KF_FLAG_DEFAULT, None)
            .expect("Windows Local AppData directory");
        let directory = PathBuf::from(OsString::from_wide(path.as_wide()));
        CoTaskMemFree(Some(path.0.cast()));
        directory.join("Carlitos")
    }
}
