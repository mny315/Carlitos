use super::{Book, BookTags, Id, Library, Target, local_path, ordering::compare_digit_runs};
use std::cmp::Ordering;

impl Library {
    /// Include nested folders and overlapping imports, counting each book once.
    pub fn source_book_counts(&self) -> std::collections::HashMap<Id, usize> {
        use std::collections::{HashMap, HashSet};
        let paths: HashMap<_, _> = self
            .sources
            .iter()
            .filter_map(|s| local_path(&s.uri).map(|p| (s.id, p)))
            .collect();
        let roots: HashMap<_, _> = paths
            .iter()
            .map(|(id, path)| (path.as_path(), *id))
            .collect();
        let mut members: HashMap<Id, HashSet<Id>> = HashMap::new();
        for book in &self.books {
            members.entry(book.source_id).or_default().insert(book.id);
        }
        let mut books_by_file: HashMap<Id, Vec<Id>> = HashMap::new();
        for part in &self.parts {
            books_by_file
                .entry(part.file_id)
                .or_default()
                .push(part.book_id);
        }
        for (source, file) in &self.source_files {
            if let Some(books) = books_by_file.get(file) {
                members
                    .entry(*source)
                    .or_default()
                    .extend(books.iter().copied());
            }
        }
        let mut include = |book, path: &std::path::Path| {
            for ancestor in path.ancestors() {
                if let Some(source) = roots.get(ancestor) {
                    members.entry(*source).or_default().insert(book);
                }
            }
        };
        for book in &self.books {
            if let Some(path) = paths.get(&book.source_id) {
                include(book.id, path);
            }
        }
        let files: HashMap<_, _> = self
            .media
            .iter()
            .filter_map(|m| local_path(&m.uri).map(|p| (m.id, p)))
            .collect();
        for part in &self.parts {
            if let Some(path) = files.get(&part.file_id) {
                include(part.book_id, path);
            }
        }
        self.sources
            .iter()
            .map(|s| (s.id, members.get(&s.id).map_or(0, HashSet::len)))
            .collect()
    }
    pub fn book_tags(&self) -> std::collections::HashMap<Id, BookTags> {
        let files: std::collections::HashMap<_, _> = self.media.iter().map(|m| (m.id, m)).collect();
        let mut parts: Vec<_> = self.parts.iter().collect();
        parts.sort_by_key(|p| (p.book_id, p.ordinal));
        let mut tags = std::collections::HashMap::<Id, BookTags>::new();
        for part in parts {
            if let Some(file) = files.get(&part.file_id) {
                let book = tags.entry(part.book_id).or_default();
                book.year = book.year.or(file.year);
                if book.genre.is_empty() {
                    book.genre = file.genre.trim().to_owned();
                }
            }
        }
        tags
    }

    /// The current book stays first, including while paused. Missing metadata
    /// sorts last; titles and IDs make the remaining order stable.
    pub fn sorted_books(&self, sort: &str) -> Vec<&Book> {
        let current = self
            .session
            .current
            .as_ref()
            .and_then(|Target::Book(id)| self.part(*id).map(|part| part.book_id));
        let tags = if matches!(sort, "year" | "genre") {
            self.book_tags()
        } else {
            Default::default()
        };
        let progress: std::collections::HashMap<_, _> =
            self.progress.iter().map(|p| (p.book_id, p)).collect();
        // Normalize once per book instead of allocating for every comparison.
        let mut books: Vec<_> = self
            .books
            .iter()
            .map(|book| {
                let text = match sort {
                    "author" => book.author.to_lowercase(),
                    "genre" => tags
                        .get(&book.id)
                        .map_or_else(String::new, |t| t.genre.to_lowercase()),
                    _ => String::new(),
                };
                (book, book.title.to_lowercase(), text)
            })
            .collect();
        let compare_text = |a: &str, b: &str| {
            a.trim()
                .is_empty()
                .cmp(&b.trim().is_empty())
                .then_with(|| compare_digit_runs(a, b))
                .then_with(|| a.cmp(b))
        };
        let listening = |id| match progress.get(&id) {
            Some(p) if !p.completed => 0,
            None => 1,
            _ => 2,
        };
        books.sort_by(|(a, a_title, a_text), (b, b_title, b_text)| {
            let order = match sort {
                "author" | "genre" => compare_text(a_text, b_text),
                "year" => {
                    let a = tags.get(&a.id).and_then(|t| t.year);
                    let b = tags.get(&b.id).and_then(|t| t.year);
                    a.is_none().cmp(&b.is_none()).then_with(|| a.cmp(&b))
                }
                "title" => Ordering::Equal,
                _ => listening(a.id).cmp(&listening(b.id)),
            };
            (Some(b.id) == current)
                .cmp(&(Some(a.id) == current))
                .then(order)
                .then_with(|| compare_text(a_title, b_title))
                .then(a.id.cmp(&b.id))
        });
        books.into_iter().map(|(book, _, _)| book).collect()
    }
}
