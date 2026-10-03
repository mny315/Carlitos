use crate::library::Media;
use std::path::{Path, PathBuf};

/// Group by the containing book folder, including nested author/series folders.
/// Numbered disc/part folders belong to their parent book.
pub(super) fn book_root(root: &Path, file: &Path) -> PathBuf {
    let mut book = root.to_path_buf();
    if let Some(parent) = file.parent()
        && let Ok(relative) = parent.strip_prefix(root)
    {
        for component in relative.components() {
            if is_part_folder(&component.as_os_str().to_string_lossy()) {
                break;
            }
            book.push(component);
        }
    }
    book
}

fn is_part_folder(name: &str) -> bool {
    let name = name.trim().to_lowercase();
    if ["chapters", "главы"].contains(&name.as_str()) {
        return true;
    }
    ["disc", "disk", "cd", "диск", "part", "часть"]
        .iter()
        .any(|prefix| {
            name.strip_prefix(prefix).is_some_and(|suffix| {
                let number = suffix.trim_start_matches([' ', '.', '-', '_']);
                !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit())
            })
        })
}

pub(super) fn book_metadata(files: &[Media], fallback: &str) -> (String, String) {
    let consistent = |get: fn(&Media) -> &str| {
        let first = get(&files[0]);
        if !first.is_empty() && files.iter().all(|m| get(m) == first) {
            Some(first.to_owned())
        } else {
            None
        }
    };
    let title = consistent(|m| &m.album).unwrap_or_else(|| fallback.to_owned());
    let author = consistent(|m| &m.artist).unwrap_or_default();
    (title, author)
}
