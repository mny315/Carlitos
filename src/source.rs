//! The two source locations supported by the application. Document IDs are
//! opaque: never use a content URI's display path for filesystem access.
use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub enum Location {
    File(PathBuf),
    Document(String),
}
impl From<PathBuf> for Location {
    fn from(path: PathBuf) -> Self {
        Self::File(path)
    }
}
impl Location {
    pub fn input(value: &str) -> Self {
        if value.starts_with("content://") {
            Self::Document(value.to_owned())
        } else {
            Self::File(PathBuf::from(value))
        }
    }
    pub fn uri(uri: &str) -> Result<Self> {
        if let Some(path) = crate::library::local_path(uri) {
            Ok(Self::File(path))
        } else {
            document_identity(uri).context("Unsupported source URI")?;
            Ok(Self::Document(uri.to_owned()))
        }
    }
}

/// Equivalent tree and single-document URIs have the same identity. Encoding
/// the pair as JSON avoids collisions when IDs contain separators or Unicode.
pub fn document_identity(uri: &str) -> Option<String> {
    let (authority, id) = document_parts(uri)?;
    Some(format!("saf:{}", serde_json::json!([authority, id])))
}

/// A readable location for UI labels only. ExternalStorageProvider IDs contain
/// a volume and a relative path; other providers' IDs must remain opaque.
pub fn display_path(uri: &str) -> String {
    if let Some(path) = crate::library::local_path(uri) {
        return path.display().to_string();
    }
    if let Some((authority, id)) = document_parts(uri)
        && authority == "com.android.externalstorage.documents"
        && let Some((volume, relative)) = id.split_once(':')
        && !volume.is_empty()
    {
        if volume == "primary" {
            return if relative.is_empty() { "/" } else { relative }.to_owned();
        }
        return format!("{volume}:/{relative}");
    }
    uri.to_owned()
}

fn document_parts(uri: &str) -> Option<(String, String)> {
    let uri = url::Url::parse(uri).ok()?;
    if uri.scheme() != "content" {
        return None;
    }
    uri.host_str()?;
    let authority = &uri[url::Position::BeforeUsername..url::Position::AfterPort];
    let segments: Vec<_> = uri.path_segments()?.collect();
    let id = match segments.as_slice() {
        ["document", id] | ["tree", _, "document", id] | ["tree", id] => *id,
        _ => return None,
    };
    // Percent decoding only; '+' is a literal character in URI path segments.
    let decoded = url::form_urlencoded::parse(
        format!("id={}", id.replace('+', "%2B").replace('&', "%26")).as_bytes(),
    )
    .next()?
    .1
    .into_owned();
    if decoded.is_empty() {
        return None;
    }
    Some((authority.to_owned(), decoded))
}

/// Reuse a surviving grant for a document already linked to that source.
/// This constructs a URI, never infers ancestry from an opaque document ID.
pub(crate) fn document_uri_in_source(document: &str, source: &str) -> Result<String> {
    let identity = document_identity(document).context("Invalid document URI")?;
    let pair: Vec<String> = serde_json::from_str(identity.strip_prefix("saf:").unwrap_or(""))?;
    let mut root = url::Url::parse(source)?;
    anyhow::ensure!(
        root.scheme() == "content"
            && root[url::Position::BeforeUsername..url::Position::AfterPort] == pair[0],
        "Document provider differs"
    );
    let segments: Vec<_> = root
        .path_segments()
        .context("Invalid source URI")?
        .collect();
    if segments.first() != Some(&"tree") {
        anyhow::ensure!(
            document_identity(source).as_ref() == Some(&identity),
            "Source is not a folder grant"
        );
        return Ok(source.to_owned());
    }
    let tree = segments.get(1).context("Missing tree ID")?;
    let tree = url::form_urlencoded::parse(
        format!("id={}", tree.replace('+', "%2B").replace('&', "%26")).as_bytes(),
    )
    .next()
    .context("Invalid tree ID")?
    .1
    .into_owned();
    root.path_segments_mut()
        .map_err(|_| anyhow::anyhow!("Invalid source URI"))?
        .clear()
        .extend(["tree", &tree, "document", &pair[1]]);
    Ok(root.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn display_paths_decode_primary_paths_and_distinguish_removable_volumes() {
        let provider = "content://com.android.externalstorage.documents";
        let id = "primary%3AAudiobooks%2F%D0%9A%D0%BD%D0%B8%D0%B3%D0%B0%20%2B%201%2F100%25%20%26%20end%3D2.m4b";
        for suffix in [
            format!("document/{id}"),
            format!("tree/{id}"),
            format!("tree/primary%3AAudiobooks/document/{id}"),
        ] {
            assert_eq!(
                display_path(&format!("{provider}/{suffix}")),
                "Audiobooks/Книга + 1/100% & end=2.m4b"
            );
        }
        assert_eq!(
            display_path(&format!("{provider}/tree/ABCD-1234%3ABooks%2Fpart+1.mp3")),
            "ABCD-1234:/Books/part+1.mp3"
        );
        assert_eq!(display_path(&format!("{provider}/tree/primary%3A")), "/");
        let file = std::env::current_dir()
            .unwrap()
            .join("Audiobooks/Book + 1/part.mp3");
        assert_eq!(
            display_path(&crate::library::file_uri(&file).unwrap()),
            file.display().to_string()
        );
        for uri in [
            "content://other.provider/tree/primary%3ABooks/document/opaque%3Aid",
            "content://10@com.android.externalstorage.documents/tree/primary%3ABooks",
            "content://com.android.externalstorage.documents/document/opaque",
            "content://com.android.externalstorage.documents/other/primary%3ABooks",
            "/Audiobooks/Book + 1",
        ] {
            assert_eq!(display_path(uri), uri);
        }
    }

    #[test]
    fn document_ids_ignore_grant_tree_and_preserve_opaque_characters() {
        let single = "content://provider/document/disk%3Abook%2F%D0%B0%2B1";
        let parent = "content://provider/tree/disk%3A/document/disk%3Abook%2F%D0%B0%2B1";
        let child = "content://provider/tree/disk%3Abook/document/disk%3Abook%2F%D0%B0+1";
        assert_eq!(document_identity(single), document_identity(parent));
        assert_eq!(document_identity(single), document_identity(child));
        assert_ne!(
            document_identity(single),
            document_identity(&single.replace("provider", "other"))
        );
        assert_eq!(
            document_identity("content://provider/document/a&b=c+1"),
            document_identity("content://provider/document/a%26b%3Dc%2B1")
        );
        let rebound = document_uri_in_source(
            "content://provider/document/a%26b%3Dc%2B1",
            "content://provider/tree/root%26plus%2B/document/folder",
        )
        .unwrap();
        assert_eq!(
            document_identity(&rebound),
            document_identity("content://provider/document/a%26b%3Dc%2B1")
        );
        assert_ne!(
            document_identity(single),
            document_identity(&single.replace("provider", "10@provider"))
        );
        assert!(crate::library::local_path(single).is_none());
        assert!(document_identity("file:///document/a").is_none());
        assert!(document_identity("content://provider/other/a").is_none());
    }
}
