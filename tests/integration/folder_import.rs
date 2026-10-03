#[path = "folder_import/reimport.rs"]
mod reimport;
#[path = "folder_import/relocation.rs"]
mod relocation;
#[path = "folder_import/scanning.rs"]
mod scanning;

use carlitos::{
    import::{ImportMode, ScanControl, scan},
    library::{Chapter, Target, file_uri, local_path},
    storage::Store,
};
use std::path::Path;

fn tempdir() -> std::io::Result<tempfile::TempDir> {
    // Match the canonical paths returned by import/relocation even when TEMP
    // uses a Windows 8.3 name or TMPDIR points through a directory symlink.
    tempfile::tempdir_in(dunce::canonicalize(std::env::temp_dir())?)
}

fn audio(root: &Path, relative: &str) -> anyhow::Result<()> {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().unwrap())?;
    let mut wav = Vec::from(&b"RIFF"[..]);
    wav.extend(1636u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(8000u32.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(1600u32.to_le_bytes());
    wav.resize(1644, 0);
    std::fs::write(path, wav)?;
    Ok(())
}
