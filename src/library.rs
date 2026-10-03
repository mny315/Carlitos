mod catalog;
mod navigation;
mod ordering;
mod paths;
#[cfg(test)]
mod tests;
mod time;

pub use ordering::{natural_cmp, order_parts};
pub use paths::{cache_dir, data_dir, file_uri, local_path};
pub use time::{book_seek, format_time, now, parse_time};

use serde::{Deserialize, Serialize};

pub type Id = i64;
pub type Millis = u64;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Media {
    pub id: Id,
    pub source_id: Id,
    pub uri: String,
    pub relative: String,
    pub identity: String,
    pub size: Option<u64>,
    pub modified: Option<i64>,
    pub duration: Option<Millis>,
    pub title: String,
    pub artist: String,
    pub album: String,
    #[serde(default)]
    pub year: Option<i32>,
    #[serde(default)]
    pub genre: String,
    #[serde(default)]
    pub sort_tags_read: bool,
    pub track: Option<u32>,
    pub disc: Option<u32>,
    pub cover: Option<String>,
    pub chapters: Vec<Chapter>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    pub start: Millis,
    pub end: Option<Millis>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub id: Id,
    pub uri: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Part {
    pub id: Id,
    pub book_id: Id,
    pub file_id: Id,
    pub title: String,
    pub ordinal: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Book {
    pub id: Id,
    pub source_id: Id,
    pub title: String,
    pub author: String,
    pub cover: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Progress {
    pub book_id: Id,
    pub part_id: Id,
    pub position: Millis,
    pub completed: bool,
    pub updated: i64,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Target {
    Book(Id),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Session {
    pub current: Option<Target>,
    pub position: Millis,
    pub volume: f64,
    pub muted: bool,
}
impl Default for Session {
    fn default() -> Self {
        Self {
            current: None,
            position: 0,
            volume: 0.7,
            muted: false,
        }
    }
}
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub sources: Vec<Source>,
    pub source_files: Vec<(Id, Id)>,
    pub media: Vec<Media>,
    pub books: Vec<Book>,
    pub parts: Vec<Part>,
    pub progress: Vec<Progress>,
    pub session: Session,
}

#[derive(Clone, Debug, Default)]
pub struct BookTags {
    pub year: Option<i32>,
    pub genre: String,
}
