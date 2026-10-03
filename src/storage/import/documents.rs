use crate::{source::document_identity, storage::Draft};
use anyhow::{Result, ensure};
use std::collections::HashMap;

/// Historical records may share an old name. A single reviewed source listing
/// must still map each current name to one recording and each document to one
/// path. Check the whole preview, including repeated/aliased source roots.
pub(super) fn validate_paths(drafts: &[Draft]) -> Result<()> {
    let mut paths = HashMap::new();
    let mut documents = HashMap::new();
    for draft in drafts.iter().filter(|draft| draft.include) {
        let root = document_identity(&draft.root_uri).unwrap_or_else(|| draft.root_uri.clone());
        for file in &draft.files {
            let document = document_identity(&file.uri);
            let identity = document.as_deref().unwrap_or(&file.identity);
            if let Some(previous) =
                paths.insert((root.clone(), &file.relative), identity.to_owned())
            {
                ensure!(
                    previous == identity,
                    "Ambiguous source path: {}",
                    file.relative
                );
            }
            if let Some(document) = document
                && let Some(previous) = documents.insert((root.clone(), document), &file.relative)
            {
                ensure!(previous == &file.relative, "Ambiguous document paths");
            }
        }
    }
    Ok(())
}
