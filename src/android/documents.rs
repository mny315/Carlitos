use crate::import::reader::DocumentReader;
use crate::{library::Media, source::document_identity};
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::json;
use std::io::Read;

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct Document {
    pub uri: String,
    pub name: String,
    pub directory: bool,
    pub size: Option<u64>,
    pub modified: Option<i64>,
    pub duration: Option<u64>,
}
impl Document {
    pub fn identity(&self) -> Result<String> {
        document_identity(&self.uri).context("Provider returned an invalid document URI")
    }
    pub fn media(self, relative: String) -> Result<Media> {
        let mut media = Media {
            identity: self.identity()?,
            uri: self.uri.clone(),
            title: std::path::Path::new(&self.name)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            relative,
            size: self.size,
            modified: self.modified,
            duration: self.duration,
            ..Default::default()
        };
        let mut reader = std::io::BufReader::new(open(&self.uri)?);
        crate::import::tags::read(&mut reader, &mut media);
        Ok(media)
    }
}
pub(crate) fn stat(uri: &str) -> Result<Document> {
    Ok(serde_json::from_value(super::bridge::documents(
        json!({"op":"stat", "uri":uri}),
    )?)?)
}
pub(crate) fn children(uri: &str) -> Result<Vec<Document>> {
    Ok(serde_json::from_value(super::bridge::documents(
        json!({"op":"children", "uri":uri}),
    )?)?)
}
pub(crate) fn relative(uri: &str, relative: &str) -> Result<Document> {
    Ok(serde_json::from_value(super::bridge::documents(
        json!({"op":"relative", "uri":uri,"relative":relative}),
    )?)?)
}
pub(crate) fn probe(uri: &str) -> Result<Document> {
    Ok(serde_json::from_value(super::bridge::documents(
        json!({"op":"probe", "uri":uri}),
    )?)?)
}
pub(crate) fn open(uri: &str) -> Result<DocumentReader<std::fs::File>> {
    use std::os::fd::FromRawFd;
    let result = super::bridge::documents(json!({"op":"open", "uri":uri}))?;
    let fd = i32::try_from(result["fd"].as_i64().context("Missing file descriptor")?)?;
    anyhow::ensure!(fd >= 0, "Invalid file descriptor");
    // Documents.open transfers ownership of a duplicated descriptor to Rust.
    let file = unsafe { std::fs::File::from_raw_fd(fd) };
    let offset = result["offset"]
        .as_u64()
        .context("Invalid document offset")?;
    let length = result["length"].as_u64();
    Ok(DocumentReader::new(file, offset, length)?)
}
pub(crate) fn read_cover(uri: &str) -> Result<Vec<u8>> {
    const LIMIT: u64 = 10 * 1024 * 1024;
    let reader = open(uri)?;
    anyhow::ensure!(reader.len() <= LIMIT, "Cover exceeds 10 MiB");
    let mut bytes = Vec::new();
    reader.take(LIMIT + 1).read_to_end(&mut bytes)?;
    anyhow::ensure!(bytes.len() as u64 <= LIMIT, "Cover exceeds 10 MiB");
    Ok(bytes)
}
