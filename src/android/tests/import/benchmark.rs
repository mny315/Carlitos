use super::{documents, scan};
use crate::import::ImportMode;
use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub(super) fn bulk(root: &str) -> Result<Value> {
    let started = std::time::Instant::now();
    let result = scan(root, ImportMode::Books)?;
    let elapsed = started.elapsed().as_millis();
    ensure!(
        result.issues.is_empty(),
        "Bulk scan errors: {:?}",
        result.issues
    );
    ensure!(result.drafts.len() == 1, "Bulk grouping changed");
    let files = &result.drafts[0].files;
    ensure!(
        files.len() == 437,
        "Bulk scan lost recordings: {}",
        files.len()
    );
    ensure!(files[0].cover.is_some(), "Bulk cover missing");
    for (index, file) in files.iter().enumerate() {
        ensure!(
            file.relative == format!("{}.mp3", index + 1),
            "Bulk order changed"
        );
        ensure!(
            file.duration.is_some_and(|d| (900..=1400).contains(&d)),
            "Bulk duration missing"
        );
        ensure!(file.cover == files[0].cover, "Bulk cover was not shared");
    }
    Ok(json!({"scan_ms": elapsed}))
}

// Read only: measure the same per-file work as the preview without importing
// anything into the user's library. Opt in with the instrumentation arguments.
pub(super) fn run(root: &str) -> Result<Value> {
    let started = std::time::Instant::now();
    let scanned = scan(root, ImportMode::Books)?;
    let scan_ms = started.elapsed().as_millis();
    let started = std::time::Instant::now();
    let mut stack = vec![documents::stat(root)?];
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    while let Some(doc) = stack.pop() {
        if !seen.insert(doc.identity()?) {
            continue;
        }
        if doc.directory {
            stack.extend(documents::children(&doc.uri)?);
        } else if crate::import::is_audio(std::path::Path::new(&doc.name)) {
            files.push(doc);
        }
    }
    let enumeration_ms = started.elapsed().as_millis();
    let mut results = Vec::new();
    for doc in files {
        let start = std::time::Instant::now();
        let probed = documents::probe(&doc.uri)?;
        let probe_ms = start.elapsed().as_millis();
        let start = std::time::Instant::now();
        let media = probed.media(doc.name.clone())?;
        results.push(json!({"name": doc.name, "probe_ms": probe_ms,
            "metadata_ms": start.elapsed().as_millis(), "duration": media.duration,
            "cover": media.cover.is_some(), "chapters": media.chapters.len()}));
    }
    let total_ms = started.elapsed().as_millis();
    Ok(
        json!({"enumeration_ms": enumeration_ms, "total_ms": total_ms, "files": results,
        "scan_ms": scan_ms, "books": scanned.drafts.len(),
        "recordings": scanned.drafts.iter().map(|draft| draft.files.len()).sum::<usize>(),
        "issues": scanned.issues}),
    )
}
