use super::View;
use crate::library::{Id, Library, file_uri, local_path, natural_cmp};
use crate::source::Location;
use crate::{SourceItem, State, app::Command};
use slint::{ComponentHandle, Model};
use std::collections::{BTreeMap, BTreeSet};

pub(super) struct SourceGroup {
    key: String,
    uri: String,
    members: Vec<Id>,
    books: Vec<Id>,
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
    let mut grouped = BTreeMap::<Id, Vec<Id>>::new();
    for source in &library.sources {
        let parent = library
            .sources
            .iter()
            .filter(|candidate| {
                if candidate.id == source.id {
                    return true;
                }
                if let (Some(root), Some(child)) = (paths.get(&candidate.id), paths.get(&source.id))
                {
                    return child.starts_with(root);
                }
                !owns_book(candidate.id)
                    && books.get(&source.id).is_some_and(|children| {
                        !children.is_empty()
                            && books
                                .get(&candidate.id)
                                .is_some_and(|parent| children.is_subset(parent))
                    })
            })
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
        let source = library
            .sources
            .iter()
            .find(|source| source.id == id)
            .unwrap();
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
    for group in &mut result {
        let ids: BTreeSet<_> = group
            .members
            .iter()
            .filter_map(|source| books.get(source))
            .flatten()
            .copied()
            .collect();
        let mut members: Vec<_> = library
            .books
            .iter()
            .filter(|book| ids.contains(&book.id))
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
    pub(super) fn refresh_sources(&mut self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let state = window.global::<State>();
        let had_selection = !self.selected_sources.is_empty();
        self.selected_sources
            .retain(|id| self.library.sources.iter().any(|s| s.id == *id));
        self.source_groups = groups(&self.library);
        self.expanded_sources
            .retain(|uri| self.source_groups.iter().any(|group| &group.uri == uri));
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
                    if let Some(book) = self.library.books.iter().find(|book| book.id == *id) {
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
                self.pending_source_removal = self
                    .library
                    .sources
                    .iter()
                    .filter(|s| self.selected_sources.contains(&s.id))
                    .map(|s| s.id)
                    .collect();
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
