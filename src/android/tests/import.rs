//! Explicit device-test feature only; never linked into the normal APK.
#[path = "import/benchmark.rs"]
mod benchmark;

use super::documents;
use crate::{
    import::{ImportMode, ScanControl, scan_location},
    library::*,
    source::Location,
    storage::Store,
};
use anyhow::{Context, Result, ensure};
use jni::{
    JNIEnv,
    objects::{JClass, JString},
    sys::jstring,
};
use serde_json::{Value, json};

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_mny315_carlitos_ImportSuite_nativeSuite(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    request: JString<'_>,
) -> jstring {
    let result = (|| -> Result<Value> {
        let request: Value = serde_json::from_str(&String::from(env.get_string(&request)?))?;
        suite(&request)
    })();
    let value = match result {
        Ok(value) => value,
        Err(error) => json!({"error": format!("{error:#}")}),
    };
    match env.new_string(value.to_string()) {
        Ok(s) => s.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}
fn snapshot(library: &Library) -> Value {
    json!({"sources": library.sources, "media": library.media, "books": library.books,
        "parts": library.parts, "progress": library.progress, "session": library.session,
        "links": library.source_files})
}
fn stable(library: &Library) -> Value {
    json!({"books": library.books, "parts": library.parts, "progress": library.progress,
        "session": library.session, "files": library.media.iter().map(|m| m.id).collect::<Vec<_>>()})
}
fn scan(uri: &str, mode: ImportMode) -> Result<crate::import::ScanResult> {
    scan_location(Location::Document(uri.into()), mode, ScanControl::default())
}
fn suite(request: &Value) -> Result<Value> {
    if let Some(root) = request["bulk"].as_str() {
        return benchmark::bulk(root);
    }
    if request["benchmark"].as_bool() == Some(true) {
        return benchmark::run(request["root"].as_str().context("Missing benchmark root")?);
    }
    if let Some(uri) = request["document"].as_str() {
        let document = documents::probe(uri)?;
        return Ok(serde_json::to_value(document.media("slice".into())?)?);
    }
    let root = request["root"].as_str().context("Missing fixture root")?;
    let result = scan(root, ImportMode::Books)?;
    ensure!(
        result.drafts.len() == 3,
        "Expected 3 books, got {}: {:?}",
        result.drafts.len(),
        result.issues
    );
    ensure!(
        result.issues.len() == 1 && result.issues[0].contains("Повреждённый"),
        "Damage isolation: {:?}",
        result.issues
    );
    let tagged = result
        .drafts
        .iter()
        .find(|d| d.title == "Книга с тегами")
        .context("Album not read")?;
    ensure!(tagged.author == "Автор Кириллица", "Artist not read");
    ensure!(
        tagged
            .files
            .iter()
            .map(|m| m.title.as_str())
            .collect::<Vec<_>>()
            == ["Глава 1.1", "Глава 1.2", "Глава 2.1"],
        "Disc/track order"
    );
    ensure!(
        tagged
            .files
            .iter()
            .all(|m| m.year == Some(2024) && m.genre == "Аудиокнига" && m.cover.is_some()),
        "Tags/embedded cover missing"
    );
    ensure!(
        tagged
            .files
            .iter()
            .all(|m| m.cover == tagged.files[0].cover),
        "Identical embedded artwork should share one cached thumbnail"
    );
    let natural = result
        .drafts
        .iter()
        .find(|d| d.title == "Естественный порядок")
        .context("Natural book missing")?;
    ensure!(
        natural
            .files
            .iter()
            .map(|m| m.title.as_str())
            .collect::<Vec<_>>()
            == ["1", "2", "10"],
        "Natural order"
    );
    ensure!(
        natural.files.iter().all(|m| m
            .cover
            .as_ref()
            .is_some_and(|p| std::path::Path::new(p).is_file())),
        "External cover missing"
    );
    let media: Vec<_> = result
        .drafts
        .iter()
        .flat_map(|d| d.files.iter().cloned())
        .collect();
    ensure!(media.len() == 14, "Expected 14 files, got {}", media.len());
    for file in &media {
        ensure!(
            file.duration.is_some_and(|d| (900..=1400).contains(&d)),
            "Duration {}: {:?}",
            file.relative,
            file.duration
        );
        if file.relative.ends_with(".m4b") {
            ensure!(
                file.chapters.len() == 2,
                "Chapters missing: {}",
                file.relative
            );
            ensure!(
                file.chapters[0].title == "Начало"
                    && file.chapters[0].end == Some(500)
                    && file.chapters[1].start == 500
                    && file.chapters[1].end == file.duration,
                "Chapter times: {}",
                file.relative
            );
        }
    }
    let temp = tempfile::tempdir_in(&super::context().cache)?;
    let database = temp.path().join("import.sqlite3");
    let mut store = Store::open(&database)?;
    let mut library = store.import(result.drafts.clone())?;
    ensure!(
        library.media.len() == 14 && library.books.len() == 3 && library.parts.len() == 14,
        "Initial import counts"
    );
    let book = library
        .books
        .iter()
        .find(|b| b.title == "Книга с тегами")
        .context("Stored book missing")?
        .clone();
    let part = library
        .parts
        .iter()
        .find(|p| p.book_id == book.id)
        .context("Stored part missing")?
        .id;
    library.session.current = Some(Target::Book(part));
    library.session.position = 640;
    library.progress.push(Progress {
        book_id: book.id,
        part_id: part,
        position: 640,
        completed: false,
        updated: 123,
    });
    store.save(&library.session, &library.progress)?;
    let before = stable(&store.load()?);
    let repeated = store.import(result.drafts)?;
    ensure!(
        before == stable(&repeated),
        "Repeat import changed IDs/progress"
    );
    let book_root = repeated
        .sources
        .iter()
        .find(|s| s.id == book.source_id)
        .context("Book source missing")?
        .uri
        .clone();
    let mut child_ids = Vec::new();
    for name in ["CD 1", "CD 2"] {
        let child = documents::relative(&book_root, name)?;
        let child_library = store.import(scan(&child.uri, ImportMode::Book)?.drafts)?;
        ensure!(
            stable(&child_library) == before,
            "Overlapping child import changed IDs/progress"
        );
        child_ids.push(
            child_library
                .sources
                .iter()
                .find(|s| {
                    crate::source::document_identity(&s.uri)
                        == crate::source::document_identity(&child.uri)
                })
                .context("Child source missing")?
                .id,
        );
    }
    let single = store.import(scan(&media[0].uri, ImportMode::Book)?.drafts)?;
    ensure!(
        stable(&single) == before,
        "Overlapping file import changed IDs/progress"
    );
    child_ids.push(
        single
            .sources
            .iter()
            .find(|s| {
                crate::source::document_identity(&s.uri)
                    == crate::source::document_identity(&media[0].uri)
            })
            .context("Single file source missing")?
            .id,
    );
    let cancelled = ScanControl::default();
    cancelled.cancel();
    ensure!(
        scan_location(
            Location::Document(root.into()),
            ImportMode::Books,
            cancelled.clone()
        )
        .is_err(),
        "Scan cancellation"
    );
    ensure!(
        store
            .relocate_location(
                book.source_id,
                &Location::Document(book_root.clone()),
                &cancelled
            )
            .is_err(),
        "Relocation cancellation"
    );
    if let Some(bad) = request["bad"].as_str() {
        let before_error = snapshot(&store.load()?);
        ensure!(
            store
                .relocate_location(
                    book.source_id,
                    &Location::Document(bad.into()),
                    &ScanControl::default()
                )
                .is_err(),
            "Bad relocation accepted"
        );
        ensure!(
            snapshot(&store.load()?) == before_error,
            "Failed relocation changed data"
        );
    }
    let destination = request["moved"].as_str().unwrap_or(&book_root);
    let relocated = store.relocate_location(
        book.source_id,
        &Location::Document(destination.into()),
        &ScanControl::default(),
    )?;
    ensure!(
        stable(&relocated) == before,
        "Relocation changed IDs/progress"
    );
    for id in &child_ids {
        let source = relocated
            .sources
            .iter()
            .find(|s| s.id == *id)
            .context("Relocated child source missing")?;
        documents::stat(&source.uri)?;
        ensure!(
            stable(&store.import(scan(&source.uri, ImportMode::Book)?.drafts)?) == before,
            "Child rescan changed IDs/progress"
        );
    }
    ensure!(
        stable(&store.import(scan(destination, ImportMode::Book)?.drafts)?) == before,
        "Parent rescan changed IDs/progress"
    );
    for id in child_ids {
        ensure!(
            stable(&store.remove_source(id)?) == before,
            "Removing overlap changed IDs/progress"
        );
    }
    drop(store);
    let mut store = Store::open(&database)?;
    ensure!(stable(&store.load()?) == before, "Reopen lost progress");
    let unrelated = store
        .load()?
        .books
        .iter()
        .find(|b| b.title == "Естественный порядок")
        .context("Other book missing")?
        .source_id;
    let removed = store.remove_source(unrelated)?;
    ensure!(
        removed.books.len() == 2
            && removed.media.len() == 11
            && removed.progress[0].position == 640,
        "Source removal affected unrelated progress"
    );
    Ok(
        json!({"files": media, "checks": ["tags and covers", "Nero/QuickTime chapters", "disc/track/natural order",
        "damaged file isolation", "repeat import", "overlapping child sources", "cancel", "atomic relocation",
        "child and parent rescan", "source deletion", "reopen checkpoint 640 ms"], "library": snapshot(&removed)}),
    )
}
