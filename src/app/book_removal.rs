use super::{Controller, text};
use crate::{
    library::{Id, Media},
    source::Location,
};
use anyhow::{Context, Result, ensure};
use std::collections::BTreeSet;

impl Controller {
    pub(super) fn remove_books(&mut self, books: &[Id], delete_files: bool) -> Result<()> {
        self.save()?;
        let mut locations = Vec::new();
        if delete_files {
            ensure!(!self.options.demo, "Demo files cannot be deleted");
            let files: BTreeSet<_> = self
                .library
                .parts
                .iter()
                .filter(|p| books.contains(&p.book_id))
                .map(|p| p.file_id)
                .collect();
            let keys: BTreeSet<_> = self
                .library
                .media
                .iter()
                .filter(|m| files.contains(&m.id))
                .map(|m| file_key(&m.uri))
                .collect();
            ensure!(
                !self
                    .library
                    .parts
                    .iter()
                    .filter(|p| !books.contains(&p.book_id))
                    .any(|p| self
                        .library
                        .media(p.file_id)
                        .is_some_and(|m| keys.contains(&file_key(&m.uri)))),
                "{}",
                text(
                    "Файлы используются другими книгами. Снимите галочку удаления файлов.",
                    "Files are used by other books. Uncheck file deletion."
                )
            );
            let mut seen = BTreeSet::new();
            locations = self
                .library
                .media
                .iter()
                .filter(|m| files.contains(&m.id))
                .filter(|m| seen.insert(file_key(&m.uri)))
                .map(|m| Ok((Location::uri(&m.uri)?, m.clone())))
                .collect::<Result<_>>()?;
            // Check the whole group before deleting the first file. Providers
            // can still fail midway; keep library entries and report that case.
            for (location, media) in &locations {
                check_deletion(location, media)?;
            }
            if self.active_book().is_some_and(|id| books.contains(&id)) {
                self.stop();
                self.pending_save = false;
                self.emit_library();
                self.save()?;
            }
            // Stop is queued on the audio thread. Wait for it to release the
            // recording before unlinking: Windows decoders can deny deletion
            // while their file handles are still open.
            #[cfg(any(target_os = "linux", windows))]
            self.audio.wait_for_commands()?;
        }
        let library = self.store()?.remove_books_with(books, || {
            for (deleted, (location, media)) in locations.iter().enumerate() {
                delete_file(location, media).with_context(|| format!("{} {deleted}. {}",
                    text("Уже удалено файлов:", "Files already deleted:"),
                    text("Не удалось удалить остальные файлы. Записи книг оставлены в библиотеке.",
                        "Could not delete the remaining files. Book entries were kept in the library.")))?;
            }
            Ok(())
        })?;
        self.reconcile(library);
        self.save()?;
        Ok(())
    }
}

fn file_key(uri: &str) -> String {
    crate::source::document_identity(uri).unwrap_or_else(|| {
        crate::library::local_path(uri).map_or_else(
            || uri.to_owned(),
            |path| {
                path.canonicalize()
                    .unwrap_or(path)
                    .to_string_lossy()
                    .into_owned()
            },
        )
    })
}

fn check_deletion(location: &Location, original: &Media) -> Result<()> {
    match location {
        Location::File(path) => {
            let metadata = match std::fs::symlink_metadata(path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error).with_context(|| path.display().to_string()),
            };
            ensure!(metadata.is_file(), "Expected an audio file: {}", path.display());
            ensure!(
                crate::platform::identity(path)? == original.identity
                    && original.size.is_none_or(|size| size == metadata.len())
                    && original.modified.is_none_or(|modified| modified == crate::platform::modified(&metadata))
                    && original.modified_subsec_nanos.is_none_or(|nanos| crate::platform::modified_subsec_nanos(&metadata) == Some(nanos)),
                "{}: {}",
                text("Файл изменился после импорта. Обновите источник или снимите галочку удаления файлов",
                    "File changed since import. Refresh the source or uncheck file deletion"),
                path.display()
            );
            Ok(())
        }
        #[cfg(target_os = "android")]
        Location::Document(uri) => crate::android::documents::check_deletion(uri, original).with_context(||
            text("Нет доступа к удалению файлов. Выберите папку источника заново или снимите галочку удаления файлов.",
                "Cannot delete files. Select the source folder again or uncheck file deletion.")),
        #[cfg(not(target_os = "android"))]
        Location::Document(_) => anyhow::bail!("Document sources require Android"),
    }
}

fn delete_file(location: &Location, original: &Media) -> Result<()> {
    match location {
        Location::File(path) => {
            // Group validation may have preceded a slow provider operation.
            // Recheck each local file immediately before removing its name.
            check_deletion(location, original)?;
            match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(error).with_context(|| path.display().to_string()),
            }
        }
        #[cfg(target_os = "android")]
        Location::Document(uri) => crate::android::documents::delete(uri, original),
        #[cfg(not(target_os = "android"))]
        Location::Document(_) => anyhow::bail!("Document sources require Android"),
    }
}
