use super::View;
use crate::library::{Id, Library, file_uri, local_path, natural_cmp};
use crate::source::Location;
use crate::{SourceItem, State, app::Command};
use slint::{ComponentHandle, Model};
use std::collections::{BTreeMap, BTreeSet, HashMap};

pub(super) struct SourceGroup {
    key: String,
    uri: String,
    members: Vec<Id>,
    books: Vec<Id>,
}

fn selected_sources_for_removal(groups: &[SourceGroup], selected: &BTreeSet<Id>) -> Vec<Id> {
    // A refresh can add a new member to a selected group, making its checkbox
    // unchecked. Only include folders that are actually shown as checked.
    groups
        .iter()
        .filter(|group| group.members.iter().all(|id| selected.contains(id)))
        .flat_map(|group| group.members.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn groups(library: &Library) -> Vec<SourceGroup> {
    let books = library.source_books();
    let paths: BTreeMap<_, _> = library
        .sources
        .iter()
        .filter_map(|source| Some((source.id, local_path(&source.uri)?)))
        .collect();
    let book_sources: BTreeSet<_> = library.books.iter().map(|book| book.source_id).collect();
    let owns_book = |id| book_sources.contains(&id);
    let by_source: HashMap<_, _> = library.sources.iter().map(|s| (s.id, s)).collect();
    let mut roots = BTreeMap::<_, Vec<_>>::new();
    let mut collections = HashMap::<_, Vec<_>>::new();
    for source in &library.sources {
        if let Some(path) = paths.get(&source.id) {
            roots.entry(path.as_path()).or_default().push(source);
        }
        if !owns_book(source.id) {
            for book in books.get(&source.id).into_iter().flatten() {
                collections.entry(*book).or_default().push(source);
            }
        }
    }
    let mut grouped = BTreeMap::<Id, Vec<Id>>::new();
    for source in &library.sources {
        let mut candidates = vec![source];
        // Local parents must be actual path ancestors. For opaque provider IDs,
        // a containing collection must include any one of the child's books.
        // These indexes avoid comparing every source with every other source.
        if let Some(path) = paths.get(&source.id) {
            for ancestor in path.ancestors() {
                if let Some(parents) = roots.get(ancestor) {
                    candidates.extend(parents.iter().copied());
                }
            }
        }
        if let Some(children) = books.get(&source.id)
            && let Some(first) = children.iter().next()
        {
            for &candidate in collections.get(first).into_iter().flatten() {
                if paths.contains_key(&source.id) && paths.contains_key(&candidate.id) {
                    continue;
                }
                if books
                    .get(&candidate.id)
                    .is_some_and(|parent| children.is_subset(parent))
                {
                    candidates.push(candidate);
                }
            }
        }
        let parent = candidates
            .into_iter()
            .max_by_key(|candidate| {
                (
                    books.get(&candidate.id).map_or(0, |books| books.len()),
                    !owns_book(candidate.id),
                    std::cmp::Reverse(
                        paths
                            .get(&candidate.id)
                            .map_or(usize::MAX, |path| path.components().count()),
                    ),
                    library.folder_sources.contains(&candidate.id),
                    std::cmp::Reverse(candidate.id),
                )
            })
            .unwrap();
        grouped.entry(parent.id).or_default().push(source.id);
    }
    let mut result = Vec::new();
    for (id, members) in grouped {
        let source = by_source[&id];
        result.push(SourceGroup {
            key: id.to_string(),
            uri: source.uri.clone(),
            members,
            books: vec![],
        });
    }
    // Older imports kept only the individual book folders. Recover their
    // shared collection without writing to the database or guessing SAF IDs.
    let local: Vec<_> = result
        .iter()
        .enumerate()
        .filter(|(_, group)| {
            !group
                .members
                .iter()
                .any(|id| library.folder_sources.contains(id))
        })
        .filter_map(|(index, group)| Some((index, local_path(&group.uri)?)))
        .collect();
    if local.len() > 1 {
        let mut root = local[0].1.clone();
        for (_, path) in &local[1..] {
            while !path.starts_with(&root) {
                if !root.pop() {
                    break;
                }
            }
        }
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from);
        let usable = root.file_name().is_some()
            && root.parent().is_some()
            && !home.as_ref().is_some_and(|home| home.starts_with(&root));
        if usable && let Ok(uri) = file_uri(&root) {
            let indices: BTreeSet<_> = local.iter().map(|(index, _)| *index).collect();
            let existing = result
                .iter()
                .find(|group| local_path(&group.uri).as_ref() == Some(&root))
                .map(|group| group.key.clone());
            let members = result
                .iter()
                .enumerate()
                .filter(|(index, _)| indices.contains(index))
                .flat_map(|(_, group)| group.members.iter().copied())
                .collect();
            result = result
                .into_iter()
                .enumerate()
                .filter(|(index, _)| !indices.contains(index))
                .map(|(_, group)| group)
                .collect();
            result.push(SourceGroup {
                key: existing.unwrap_or_else(|| uri.clone()),
                uri,
                members,
                books: vec![],
            });
        }
    }
    let by_book: HashMap<_, _> = library.books.iter().map(|book| (book.id, book)).collect();
    for group in &mut result {
        let ids: BTreeSet<_> = group
            .members
            .iter()
            .filter_map(|source| books.get(source))
            .flatten()
            .copied()
            .collect();
        let mut members: Vec<_> = ids
            .iter()
            .filter_map(|id| by_book.get(id).copied())
            .collect();
        members.sort_by(|a, b| natural_cmp(&a.title, &b.title).then(a.id.cmp(&b.id)));
        group.books = members.into_iter().map(|book| book.id).collect();
    }
    result.sort_by(|a, b| {
        natural_cmp(
            &crate::source::display_path(&a.uri),
            &crate::source::display_path(&b.uri),
        )
    });
    result
}

impl View {
    #[cfg(all(debug_assertions, target_os = "linux"))]
    pub fn test_restore_source_selection(&self, target: &mut Self) {
        target.restore_source_selection(self.save_source_selection());
    }

    #[cfg(any(target_os = "android", all(debug_assertions, target_os = "linux")))]
    pub(crate) fn save_source_selection(&self) -> BTreeSet<Id> {
        self.selected_sources.clone()
    }

    #[cfg(any(target_os = "android", all(debug_assertions, target_os = "linux")))]
    pub(crate) fn restore_source_selection(&mut self, saved: BTreeSet<Id>) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        self.selected_sources = saved;
        window
            .global::<State>()
            .set_source_selection(!self.library.sources.is_empty());
        // Revalidate the exact saved IDs against complete current groups.
        // Restoring a checked row key would also select newly added members.
        self.refresh_sources();
    }

    pub(super) fn refresh_sources(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<State>();
        let had_selection = !self.selected_sources.is_empty();
        let sources: BTreeSet<_> = self.library.sources.iter().map(|s| s.id).collect();
        self.selected_sources.retain(|id| sources.contains(id));
        self.source_groups = groups(&self.library);
        self.selected_sources =
            selected_sources_for_removal(&self.source_groups, &self.selected_sources)
                .into_iter()
                .collect();
        let uris: BTreeSet<_> = self
            .source_groups
            .iter()
            .map(|group| group.uri.as_str())
            .collect();
        self.expanded_sources
            .retain(|uri| uris.contains(uri.as_str()));
        let books: HashMap<_, _> = self
            .library
            .books
            .iter()
            .map(|book| (book.id, book))
            .collect();
        let mut rows = Vec::new();
        let mut selected = 0;
        for group in &self.source_groups {
            let path = crate::source::display_path(&group.uri);
            let title = if path.starts_with("content://") {
                super::super::text("Папка с книгами", "Book folder").to_owned()
            } else {
                std::path::Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| path.clone())
            };
            let checked = group
                .members
                .iter()
                .all(|id| self.selected_sources.contains(id));
            selected += i32::from(checked);
            let expanded = self.expanded_sources.contains(&group.uri);
            rows.push(SourceItem {
                key: group.key.clone().into(),
                title: title.into(),
                path: path.into(),
                detail: crate::i18n::book_count(group.books.len()).into(),
                selected: checked,
                expanded,
                ..Default::default()
            });
            if (expanded || self.closing_sources.contains(&group.uri))
                && !state.get_source_selection()
            {
                for id in &group.books {
                    if let Some(book) = books.get(id) {
                        rows.push(SourceItem {
                            key: format!("{}:{id}", group.key).into(),
                            title: book.title.clone().into(),
                            detail: book.author.clone().into(),
                            child: true,
                            expanded,
                            animating: self.opening_sources.contains(&group.uri),
                            book_key: id.to_string().into(),
                            ..Default::default()
                        });
                    }
                }
            }
        }
        // Preserve existing components so disclosures can reverse smoothly.
        let keys: BTreeSet<_> = rows.iter().map(|row| row.key.clone()).collect();
        for index in (0..self.sources.row_count()).rev() {
            if self
                .sources
                .row_data(index)
                .is_some_and(|row| !keys.contains(&row.key))
            {
                self.sources.remove(index);
            }
        }
        for (index, row) in rows.into_iter().enumerate() {
            if self
                .sources
                .row_data(index)
                .as_ref()
                .is_some_and(|old| old.key == row.key)
            {
                if self.sources.row_data(index).as_ref() != Some(&row) {
                    self.sources.set_row_data(index, row);
                }
            } else {
                if let Some(old) = (index..self.sources.row_count()).find(|old| {
                    self.sources
                        .row_data(*old)
                        .is_some_and(|old| old.key == row.key)
                }) {
                    self.sources.remove(old);
                }
                self.sources.insert(index, row);
            }
        }
        state.set_source_folder_count(self.source_groups.len() as i32);
        state.set_selected_source_count(selected);
        if self.library.sources.is_empty() || (had_selection && self.selected_sources.is_empty()) {
            state.set_source_selection(false);
        }
    }

    pub(super) fn folder_action(&mut self, action: &str, key: &str) -> Option<Command> {
        let window = self.window.upgrade()?;
        let state = window.global::<State>();
        if action == "finish-source-transition" {
            self.opening_sources.clear();
            self.closing_sources.clear();
            self.refresh_sources();
            return None;
        }
        let group = self.source_groups.iter().find(|group| group.key == key)?;
        match action {
            "toggle-source" => {
                if !self.expanded_sources.insert(group.uri.clone()) {
                    self.expanded_sources.remove(&group.uri);
                    self.opening_sources.remove(&group.uri);
                    self.closing_sources.insert(group.uri.clone());
                } else {
                    self.closing_sources.remove(&group.uri);
                    self.opening_sources.insert(group.uri.clone());
                }
                self.refresh_sources();
                let weak = window.as_weak();
                self.source_transition_timer.start(
                    slint::TimerMode::SingleShot,
                    std::time::Duration::from_millis(240),
                    move || {
                        if let Some(window) = weak.upgrade() {
                            window
                                .global::<State>()
                                .invoke_action("finish-source-transition".into(), "".into());
                        }
                    },
                );
            }
            "rescan" if !state.get_source_busy() => {
                state.set_source_update_path(crate::source::display_path(&group.uri).into());
                return Some(Command::UpdateFolder(Location::uri(&group.uri).ok()?, None));
            }
            "begin-source-edit" => {
                super::renew_picker_owner(&window);
                state.set_relocate_key(key.into());
                state.set_relocate_path(match Location::uri(&group.uri).ok()? {
                    Location::File(path) => path.to_string_lossy().into_owned().into(),
                    Location::Document(uri) => uri.into(),
                });
                state.set_source_error("".into());
                state.set_overlay(5);
            }
            "update-source" if !state.get_source_busy() => {
                state.set_source_update_path(crate::source::display_path(&group.uri).into());
                let mut replacement = Location::input(state.get_relocate_path().as_str());
                if let Location::File(path) = &mut replacement {
                    *path = dunce::canonicalize(&*path).unwrap_or_else(|_| path.clone());
                }
                let same = match &replacement {
                    Location::File(path) => local_path(&group.uri).as_ref() == Some(path),
                    Location::Document(uri) => uri == &group.uri,
                };
                state.set_overlay(0);
                return Some(Command::UpdateFolder(
                    Location::uri(&group.uri).ok()?,
                    (!same).then_some(replacement),
                ));
            }
            "remove-source" if !state.get_source_busy() => {
                return Some(Command::RemoveSources(group.members.clone()));
            }
            _ => {}
        }
        None
    }

    pub(super) fn source_selection_action(&mut self, action: &str, arg: &str) -> Option<Command> {
        let window = self.window.upgrade()?;
        let state = window.global::<State>();
        match action {
            "select-sources" => state.set_source_selection(!self.library.sources.is_empty()),
            "select-source" => {
                let group = self.source_groups.iter().find(|group| group.key == arg)?;
                state.set_source_selection(true);
                if group
                    .members
                    .iter()
                    .all(|id| self.selected_sources.contains(id))
                {
                    for id in &group.members {
                        self.selected_sources.remove(id);
                    }
                } else {
                    self.selected_sources.extend(&group.members);
                }
            }
            "select-all-sources" => {
                self.selected_sources = self.library.sources.iter().map(|s| s.id).collect();
                state.set_source_selection(!self.selected_sources.is_empty());
            }
            "deselect-sources" => self.selected_sources.clear(),
            "clear-source-selection" => {
                self.selected_sources.clear();
                self.pending_source_removal.clear();
                state.set_source_selection(false);
            }
            "confirm-remove-sources" if !state.get_source_busy() => {
                // Freeze the IDs at confirmation time; a subsequent library
                // refresh must never make the same row index target a new source.
                self.pending_source_removal =
                    selected_sources_for_removal(&self.source_groups, &self.selected_sources);
                if !self.pending_source_removal.is_empty() {
                    state.set_remove_source_count(state.get_selected_source_count());
                    state.set_remove_kind("sources".into());
                    state.set_remove_key("".into());
                    state.set_overlay(1);
                }
            }
            "remove-sources" if !state.get_source_busy() => {
                let ids = std::mem::take(&mut self.pending_source_removal);
                return (!ids.is_empty()).then_some(Command::RemoveSources(ids));
            }
            _ => return None,
        }
        self.refresh_sources();
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_local_and_document_collections_keep_all_members_under_their_root() {
        use crate::library::{Book, Part, Source};
        const COUNT: Id = 2_000;
        for documents in [false, true] {
            let mut library = Library::default();
            let root = if cfg!(windows) {
                "file:///C:/audit-collection"
            } else {
                "file:///audit-collection"
            };
            let uri = |id| {
                if documents {
                    format!("content://provider/tree/opaque/document/{id}")
                } else if id == COUNT + 1 {
                    root.into()
                } else {
                    format!("{root}/{id}")
                }
            };
            for id in 1..=COUNT {
                library.sources.push(Source { id, uri: uri(id) });
                library.books.push(Book {
                    id,
                    source_id: id,
                    title: format!("Book {id}"),
                    author: String::new(),
                    cover: None,
                });
                library.parts.push(Part {
                    id,
                    book_id: id,
                    file_id: id,
                    title: String::new(),
                    ordinal: 0,
                });
                library.source_files.push((COUNT + 1, id));
            }
            library.sources.push(Source {
                id: COUNT + 1,
                uri: uri(COUNT + 1),
            });
            library.folder_sources.insert(COUNT + 1);
            let started = std::time::Instant::now();
            let grouped = groups(&library);
            println!(
                "Grouping {COUNT} sources (documents={documents}): {:?}",
                started.elapsed()
            );
            assert_eq!(grouped.len(), 1);
            assert_eq!(grouped[0].key, (COUNT + 1).to_string());
            assert_eq!(grouped[0].members, (1..=COUNT + 1).collect::<Vec<_>>());
            assert_eq!(grouped[0].books, (1..=COUNT).collect::<Vec<_>>());
        }
    }

    #[test]
    fn shared_books_do_not_merge_unrelated_local_paths_but_identify_opaque_collections() {
        use crate::library::{Book, Part, Source};
        let prefix = if cfg!(windows) {
            "file:///C:"
        } else {
            "file://"
        };
        let mut library = Library {
            sources: vec![
                Source {
                    id: 1,
                    uri: format!("{prefix}/independent/one"),
                },
                Source {
                    id: 2,
                    uri: format!("{prefix}/independent/two"),
                },
            ],
            books: vec![Book {
                id: 1,
                source_id: 1,
                title: "Book".into(),
                author: String::new(),
                cover: None,
            }],
            parts: vec![Part {
                id: 1,
                book_id: 1,
                file_id: 1,
                title: String::new(),
                ordinal: 0,
            }],
            source_files: vec![(2, 1)],
            folder_sources: BTreeSet::from([2]),
            ..Default::default()
        };
        let local = groups(&library);
        assert_eq!(local.len(), 2);
        assert!(
            local
                .iter()
                .all(|group| group.members.len() == 1 && group.books == [1])
        );
        library.sources[1].uri = "content://provider/tree/opaque/document/collection".into();
        let opaque = groups(&library);
        assert_eq!(opaque.len(), 1);
        assert_eq!(opaque[0].key, "2");
        assert_eq!(opaque[0].members, [1, 2]);
        assert_eq!(opaque[0].books, [1]);
    }

    #[test]
    fn refreshed_groups_cannot_delete_members_of_an_unchecked_folder() {
        // A refresh added source 3 to a previously selected two-source folder.
        // That folder is now unchecked; only the other checked folder is shown
        // as selected in the UI and may appear in the removal confirmation.
        let groups = vec![
            SourceGroup {
                key: "first".into(),
                uri: "file:///first".into(),
                members: vec![1, 2, 3],
                books: vec![],
            },
            SourceGroup {
                key: "second".into(),
                uri: "file:///second".into(),
                members: vec![4, 5],
                books: vec![],
            },
        ];
        let mut selected = BTreeSet::from([1, 2, 4, 5]);
        assert_eq!(selected_sources_for_removal(&groups, &selected), [4, 5]);
        selected.insert(3);
        assert_eq!(
            selected_sources_for_removal(&groups, &selected),
            [1, 2, 3, 4, 5]
        );
        selected.remove(&4);
        assert_eq!(selected_sources_for_removal(&groups, &selected), [1, 2, 3]);
    }
}
