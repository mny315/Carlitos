mod actions;
mod book_model;
mod books;
mod covers;
mod editor;
mod events;
mod imports;
mod playback;
mod preferences;

use super::{Command, audio::Playback};
use crate::{AppWindow, PartItem, State};
use crate::{library::*, settings::Settings, storage::Draft};
use book_model::BookModel;
pub use covers::CoverResult;
use covers::Covers;
#[cfg(target_os = "android")]
pub(crate) use editor::SavedEditor;
pub use editor::renew_picker_owner;
use slint::{ComponentHandle, ModelRc, VecModel};
use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::mpsc::Sender};

pub struct View {
    window: slint::Weak<AppWindow>,
    tx: Sender<Command>,
    library: Library,
    books: Rc<BookModel>,
    parts: Rc<VecModel<PartItem>>,
    covers: Rc<Covers>,
    by_book: HashMap<Id, Vec<usize>>,
    by_media: HashMap<Id, usize>,
    by_part: HashMap<Id, usize>,
    book_tags: HashMap<Id, BookTags>,
    cover_request: u64,
    selected: Option<Id>,
    playback: Playback,
    drafts: Vec<Draft>,
    settings: Settings,
    system_dark: Option<bool>,
    query: String,
    filter: i32,
    hidden: bool,
    active_part_row: Option<usize>,
    notice_timer: slint::Timer,
}
impl View {
    #[cfg(all(debug_assertions, target_os = "linux"))]
    pub fn test_cover(&mut self, path: String) -> bool {
        if let Some(book) = self.library.books.first_mut() {
            book.cover = Some(path.clone());
        }
        self.refresh_books();
        self.covers.image(Some(&path)).size().width > 0
    }
    pub fn new(
        window: &AppWindow,
        tx: Sender<Command>,
        settings: Settings,
    ) -> (Rc<RefCell<Self>>, async_channel::Receiver<CoverResult>) {
        let (covers, receiver) = Covers::new();
        let books = Rc::new(BookModel::new(covers.clone()));
        let parts = Rc::new(VecModel::default());
        window
            .global::<State>()
            .set_books(ModelRc::from(books.clone()));
        window.global::<State>().set_parts(parts.clone().into());
        let filter = i32::from(settings.library_tab == "started");
        window.global::<State>().set_filter(filter);
        let view = Rc::new(RefCell::new(Self {
            window: window.as_weak(),
            tx,
            library: Library::default(),
            books,
            parts,
            covers,
            by_book: HashMap::new(),
            by_media: HashMap::new(),
            by_part: HashMap::new(),
            book_tags: HashMap::new(),
            cover_request: 0,
            selected: None,
            playback: Playback::default(),
            drafts: vec![],
            settings,
            system_dark: None,
            query: String::new(),
            filter,
            hidden: false,
            active_part_row: None,
            notice_timer: slint::Timer::default(),
        }));
        view.borrow().apply_settings();
        Self::bind(&view, window);
        (view, receiver)
    }
    pub fn send(&self, command: Command) {
        let _ = self.tx.send(command);
    }
    fn expire_notice(&self) {
        let window = self.window.clone();
        self.notice_timer.start(
            slint::TimerMode::SingleShot,
            std::time::Duration::from_secs(5),
            move || {
                if let Some(window) = window.upgrade() {
                    window.global::<State>().set_notice("".into());
                }
            },
        );
    }
    pub fn close(&self, tray: bool) {
        if self.settings.close_to_tray {
            if tray {
                self.send(Command::Hidden(true));
            } else if let Some(w) = self.window.upgrade() {
                w.global::<State>().set_overlay(4);
            }
        } else {
            self.send(Command::Quit);
        }
    }
    pub fn cover(&mut self, result: CoverResult) {
        let path = result.path.clone();
        self.covers.accept(result);
        if let Some(w) = self.window.upgrade() {
            let state = w.global::<State>();
            if state.get_overlay() == 2 && state.get_edit_cover_path() == path {
                state.set_edit_cover(self.covers.image(Some(&path)));
            }
        }
        self.books.cover_changed(&path);
        self.refresh_current();
        if let Some(id) = self.selected
            && let Some(item) = self.book_item(id)
            && let Some(w) = self.window.upgrade()
        {
            w.global::<State>().set_selected(item);
        }
    }
}
