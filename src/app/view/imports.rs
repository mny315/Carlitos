use super::View;
use crate::{ImportItem, PartItem, State, library::format_time};
use slint::{ComponentHandle, VecModel};
use std::rc::Rc;

impl View {
    pub(super) fn refresh_drafts(&self) {
        let Some(window) = self.window.upgrade() else {
            return;
        };
        let rows: Vec<_> = self
            .drafts
            .iter()
            .map(|draft| ImportItem {
                title: draft.title.clone().into(),
                author: draft.author.clone().into(),
                included: draft.include,
                detail: crate::i18n::part_count(draft.files.len()).into(),
            })
            .collect();
        let state = window.global::<State>();
        let included = self
            .drafts
            .iter()
            .filter(|draft| draft.include && !draft.files.is_empty())
            .count();
        state.set_included_drafts(included as i32);
        state.set_import_summary(
            format!(
                "{} · {} {included}",
                crate::i18n::book_count(rows.len()),
                crate::app::text("выбрано", "selected")
            )
            .into(),
        );
        let path = crate::source::display_path(state.get_import_path().as_str());
        let title = if !state.get_import_folder_name().is_empty() {
            state.get_import_folder_name().to_string()
        } else if path.starts_with("content://") {
            crate::app::text("Папка с книгами", "Book folder").to_owned()
        } else {
            std::path::Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or(path)
        };
        state.set_import_folder_title(title.into());
        state.set_drafts(Rc::new(VecModel::from(rows)).into());
    }
    pub(super) fn refresh_draft(&self) {
        let Some(w) = self.window.upgrade() else {
            return;
        };
        let state = w.global::<State>();
        let rows: Vec<_> = usize::try_from(state.get_selected_draft())
            .ok()
            .and_then(|i| self.drafts.get(i))
            .map(|d| {
                d.files
                    .iter()
                    .enumerate()
                    .map(|(i, f)| PartItem {
                        title: f.relative.clone().into(),
                        detail: f.duration.map(format_time).unwrap_or_default().into(),
                        number: (i + 1).to_string().into(),
                        ..Default::default()
                    })
                    .collect()
            })
            .unwrap_or_default();
        state.set_draft_files(Rc::new(VecModel::from(rows)).into());
    }
}
