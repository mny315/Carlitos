use super::*;
use anyhow::Context;

pub fn scan(paths: Vec<PathBuf>, mode: ImportMode, control: ScanControl) -> Result<ScanResult> {
    let discoverer = discoverer()?;
    let mut result = ScanResult {
        drafts: vec![],
        issues: vec![],
    };
    for path in paths {
        control.check()?;
        let mut files = vec![];
        let root = if path.is_dir() {
            let path = dunce::canonicalize(&path)?;
            walk(&path, &control, &mut files, &mut result.issues)?;
            path
        } else {
            // Resolve directory aliases just as for a folder import, while
            // retaining the leaf so read_media still rejects symlinked files.
            let parent = path
                .parent()
                .context(crate::i18n::tr("Файл без родительской папки"))?;
            let parent = if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            };
            let root = match dunce::canonicalize(parent) {
                Ok(root) => root,
                Err(error) => {
                    result.issues.push(format!("{}: {error}", path.display()));
                    continue;
                }
            };
            files.push(
                root.join(
                    path.file_name()
                        .context(crate::i18n::tr("Файл без родительской папки"))?,
                ),
            );
            root
        };
        if mode == ImportMode::Books {
            let mut groups = std::collections::BTreeMap::<PathBuf, Vec<PathBuf>>::new();
            for file in files.into_iter().filter(|p| is_audio(p)) {
                control.check()?;
                groups
                    .entry(book_root(&root, &file))
                    .or_default()
                    .push(file);
            }
            let mut groups: Vec<_> = groups.into_iter().collect();
            if groups.is_empty() {
                result.issues.push(tformat!(
                    "{}: пригодных аудиофайлов не найдено",
                    root.display()
                ));
            }
            groups
                .sort_by(|(a, _), (b, _)| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
            let collection = groups.len() > 1;
            for (book, files) in groups {
                let before = result.drafts.len();
                add_draft(&book, files, &discoverer, &control, &mut result)?;
                if collection
                    && book == root
                    && let Some(draft) = result.drafts.get_mut(before)
                    && draft.files.len() == 1
                    && draft.files[0].album.is_empty()
                {
                    draft.title = draft.files[0].title.clone();
                }
            }
        } else {
            add_draft(&root, files, &discoverer, &control, &mut result)?;
        }
    }
    control.check()?;
    Ok(result)
}
fn walk(
    root: &Path,
    control: &ScanControl,
    files: &mut Vec<PathBuf>,
    issues: &mut Vec<String>,
) -> Result<()> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        control.check()?;
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) => {
                issues.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        for entry in entries {
            control.check()?;
            match entry {
                Ok(entry) => match entry.file_type() {
                    Ok(t) if t.is_symlink() => issues.push(tformat!(
                        "{}: символическая ссылка пропущена",
                        entry.path().display()
                    )),
                    Ok(t) if t.is_dir() => stack.push(entry.path()),
                    Ok(t) if t.is_file() => files.push(entry.path()),
                    Ok(_) => {}
                    Err(e) => issues.push(format!("{}: {e}", entry.path().display())),
                },
                Err(e) => issues.push(e.to_string()),
            }
        }
    }
    Ok(())
}
fn add_draft(
    root: &Path,
    paths: Vec<PathBuf>,
    discoverer: &Discoverer,
    control: &ScanControl,
    result: &mut ScanResult,
) -> Result<()> {
    let mut files = vec![];
    let mut seen = std::collections::HashSet::new();
    for path in paths {
        control.check()?;
        if !is_audio(&path) {
            continue;
        }
        match read_media(&path, root, discoverer) {
            Ok(media) => {
                if seen.insert(media.identity.clone()) {
                    files.push(media);
                } else {
                    result
                        .issues
                        .push(tformat!("{}: повторная ссылка на файл", path.display()));
                }
            }
            Err(e) => result.issues.push(format!("{}: {e:#}", path.display())),
        }
    }
    if files.is_empty() {
        result.issues.push(tformat!(
            "{}: пригодных аудиофайлов не найдено",
            root.display()
        ));
        return Ok(());
    }
    order_parts(&mut files);
    let (title, author) = book_metadata(
        &files,
        &root.file_name().unwrap_or_default().to_string_lossy(),
    );
    let cover = files.iter().find_map(|f| f.cover.clone()).or_else(|| {
        ["cover.jpg", "cover.png", "folder.jpg"]
            .iter()
            .find_map(|name| {
                let path = root.join(name);
                let bytes = read_cover(&path).ok()?;
                cache_cover(&bytes).ok()
            })
    });
    for file in &mut files {
        if file.cover.is_none() {
            file.cover = cover.clone();
        }
    }
    result.drafts.push(Draft {
        root_uri: file_uri(root)?,
        title,
        author,
        files,
        include: true,
    });
    Ok(())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[test]
    fn cancellation_and_symlink_cycles() -> Result<()> {
        let temp = tempfile::tempdir()?;
        std::os::unix::fs::symlink(temp.path(), temp.path().join("loop"))?;
        let c = ScanControl::default();
        let mut files = vec![];
        let mut issues = vec![];
        walk(temp.path(), &c, &mut files, &mut issues)?;
        assert!(files.is_empty());
        assert_eq!(issues.len(), 1);
        c.cancel();
        assert!(walk(temp.path(), &c, &mut files, &mut issues).is_err());
        Ok(())
    }
}
