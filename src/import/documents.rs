//! SAF enumeration uses display names only for relative grouping; every read
//! uses the provider's original URI and document ID.
use super::*;
use crate::android::documents::{self, Document};
use anyhow::Context;
use std::collections::{BTreeMap, HashSet};

pub(super) fn scan(uri: &str, mode: ImportMode, control: ScanControl) -> Result<ScanResult> {
    control.check()?;
    let root = documents::stat(uri)?;
    if !root.directory {
        let file = documents::probe(&root.uri)?.media(root.name.clone())?;
        let (title, author) = book_metadata(std::slice::from_ref(&file), &file.title);
        return Ok(ScanResult {
            drafts: vec![Draft {
                root_uri: root.uri,
                title,
                author,
                files: vec![file],
                include: true,
            }],
            issues: vec![],
        });
    }
    let mut result = ScanResult {
        drafts: vec![],
        issues: vec![],
    };
    let mut dirs = BTreeMap::from([(PathBuf::new(), root.clone())]);
    let mut stack = vec![(PathBuf::new(), root)];
    let mut visited = HashSet::new();
    let mut files = Vec::new();
    let mut covers = BTreeMap::<PathBuf, Document>::new();
    while let Some((relative, dir)) = stack.pop() {
        control.check()?;
        if !visited.insert(dir.identity()?) {
            continue;
        }
        let children = match documents::children(&dir.uri) {
            Ok(children) => children,
            Err(error) if relative.as_os_str().is_empty() => return Err(error),
            Err(error) => {
                result
                    .issues
                    .push(format!("{}: {error:#}", relative.display()));
                continue;
            }
        };
        let mut names = std::collections::HashMap::new();
        for child in &children {
            *names.entry(child.name.as_str()).or_insert(0usize) += 1;
        }
        for child in &children {
            control.check()?;
            if child.name.is_empty()
                || child.name.contains('/')
                || [".", ".."].contains(&child.name.as_str())
                || names[child.name.as_str()] != 1
            {
                result.issues.push(format!(
                    "{}: missing or ambiguous document name",
                    child.name
                ));
                continue;
            }
            let path = relative.join(&child.name);
            if child.directory {
                dirs.insert(path.clone(), child.clone());
                stack.push((path, child.clone()));
            } else if is_audio(&path) {
                files.push((path, child.clone()));
            } else if ["cover.jpg", "cover.png", "folder.jpg"].contains(&child.name.as_str()) {
                covers.insert(path, child.clone());
            }
        }
    }
    let mut groups = BTreeMap::<PathBuf, Vec<(PathBuf, Document)>>::new();
    for (path, doc) in files {
        let group = if mode == ImportMode::Book {
            PathBuf::new()
        } else {
            book_root(Path::new(""), &path)
        };
        groups.entry(group).or_default().push((path, doc));
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort_by(|(a, _), (b, _)| natural_cmp(&a.to_string_lossy(), &b.to_string_lossy()));
    let collection = groups.len() > 1;
    // Scan across book boundaries as well: a collection may contain hundreds
    // of single-file books. Keep the original order independently of which
    // provider finishes first, and bound concurrent descriptors and readers.
    let mut media = {
        let entries: Vec<_> = groups
            .iter()
            .flat_map(|(path, entries)| {
                entries
                    .iter()
                    .map(move |(relative, doc)| (path, relative, doc))
            })
            .collect();
        super::parallel::map(&entries, &control, |(path, relative, doc)| {
            control.check()?;
            let doc = documents::probe(&doc.uri)?;
            control.check()?;
            doc.media(relative.strip_prefix(path)?.to_string_lossy().into_owned())
        })?
    }
    .into_iter();
    for (path, entries) in groups {
        let root = dirs.get(&path).context("Missing document folder")?;
        let mut files = Vec::new();
        let mut seen = HashSet::new();
        for (relative, _) in entries {
            control.check()?;
            match media.next().context("Missing document scan result")? {
                Ok(media) if seen.insert(media.identity.clone()) => files.push(media),
                Ok(_) => {}
                Err(error) => result
                    .issues
                    .push(format!("{}: {error:#}", relative.display())),
            }
        }
        if !files.is_empty() {
            order_parts(&mut files);
            let fallback = if collection && path.as_os_str().is_empty() && files.len() == 1 {
                &files[0].title
            } else {
                &root.name
            };
            let (title, author) = book_metadata(&files, fallback);
            let mut cover = files.iter().find_map(|file| file.cover.clone());
            if cover.is_none() {
                for name in ["cover.jpg", "cover.png", "folder.jpg"] {
                    control.check()?;
                    if let Some(doc) = covers.get(&path.join(name)) {
                        cover = documents::read_cover(&doc.uri)
                            .ok()
                            .and_then(|bytes| cache_cover(&bytes).ok());
                        if cover.is_some() {
                            break;
                        }
                    }
                }
            }
            for file in &mut files {
                if file.cover.is_none() {
                    file.cover = cover.clone();
                }
            }
            result.drafts.push(Draft {
                root_uri: root.uri.clone(),
                title,
                author,
                files,
                include: true,
            });
        }
    }
    control.check()?;
    if result.drafts.is_empty() {
        result
            .issues
            .push(crate::i18n::tr("Пригодных аудиофайлов не найдено").into());
    }
    Ok(result)
}
