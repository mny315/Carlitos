use super::super::theme::CoverPalette;
use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet, VecDeque},
    rc::Rc,
};

pub struct CoverResult {
    pub(super) path: String,
    pixels: Option<(u32, u32, Vec<u8>)>,
    palette: CoverPalette,
}
struct CachedCover {
    image: slint::Image,
    palette: CoverPalette,
}
pub(super) struct Covers {
    images: RefCell<HashMap<String, CachedCover>>,
    order: RefCell<VecDeque<String>>,
    pending: RefCell<HashSet<String>>,
    waiting: RefCell<VecDeque<String>>,
    active: Cell<usize>,
    retry: RefCell<HashSet<String>>,
    failed: RefCell<HashSet<String>>,
    requests: std::sync::mpsc::Sender<Option<String>>,
    cancelled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    worker: RefCell<Option<std::thread::JoinHandle<()>>>,
}
impl Covers {
    pub(super) fn new() -> (Rc<Self>, async_channel::Receiver<CoverResult>) {
        let (requests, receiver) = std::sync::mpsc::channel::<Option<String>>();
        let (tx, results) = async_channel::unbounded();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stop = cancelled.clone();
        let worker = std::thread::Builder::new()
            .name("carlitos-covers".into())
            .spawn(move || {
                while let Ok(Some(path)) = receiver.recv() {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        break;
                    }
                    let pixels = crate::import::load_cover_image(std::path::Path::new(&path))
                        .ok()
                        .map(|im| {
                            let rgba = im.thumbnail(216, 292).into_rgba8();
                            (rgba.width(), rgba.height(), rgba.into_raw())
                        });
                    let palette = pixels
                        .as_ref()
                        .map_or_else(CoverPalette::default, |(_, _, bytes)| {
                            CoverPalette::from_pixels(bytes)
                        });
                    if tx
                        .send_blocking(CoverResult {
                            path,
                            pixels,
                            palette,
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            // Cover decoding is optional. A closed request channel makes
            // image()/palette() use their normal missing-cover fallback.
            .ok();
        (
            Rc::new(Self {
                images: RefCell::new(HashMap::new()),
                order: RefCell::new(VecDeque::new()),
                pending: RefCell::new(HashSet::new()),
                waiting: RefCell::new(VecDeque::new()),
                active: Cell::new(0),
                retry: RefCell::new(HashSet::new()),
                failed: RefCell::new(HashSet::new()),
                requests,
                cancelled,
                worker: RefCell::new(worker),
            }),
            results,
        )
    }
    pub(super) fn image(&self, path: Option<&str>) -> slint::Image {
        let Some(path) = path else {
            return slint::Image::default();
        };
        if let Some(cover) = self.images.borrow().get(path) {
            // Keep the current cover and its palette resident while browsing
            // a large library; eviction must not flash the neutral theme.
            let mut order = self.order.borrow_mut();
            order.retain(|p| p != path);
            order.push_back(path.into());
            return cover.image.clone();
        }
        if !self.failed.borrow().contains(path) && self.pending.borrow_mut().insert(path.into()) {
            self.waiting.borrow_mut().push_back(path.into());
            self.dispatch();
        }
        slint::Image::default()
    }
    pub(super) fn palette(&self, path: Option<&str>) -> Option<CoverPalette> {
        match path {
            None => Some(CoverPalette::default()),
            Some(path) if self.failed.borrow().contains(path) => Some(CoverPalette::default()),
            Some(path) => self.images.borrow().get(path).map(|cover| cover.palette),
        }
    }
    pub(super) fn clear_failures(&self) {
        self.failed.borrow_mut().clear();
    }

    fn dispatch(&self) {
        // Limit decoded images awaiting the UI without dropping requests made
        // while the worker is busy (for example during fast scrolling).
        while self.active.get() < 64 {
            let Some(path) = self.waiting.borrow_mut().pop_front() else {
                break;
            };
            if self.requests.send(Some(path.clone())).is_err() {
                self.pending.borrow_mut().remove(&path);
                self.failed.borrow_mut().insert(path);
                continue;
            }
            self.active.set(self.active.get() + 1);
        }
    }
    pub(super) fn retry(&self, path: &str) {
        self.failed.borrow_mut().remove(path);
        if self.pending.borrow().contains(path) {
            // A result decoded before the repair may still be queued.
            self.retry.borrow_mut().insert(path.into());
        }
    }
    pub(super) fn accept(&self, result: CoverResult) {
        self.pending.borrow_mut().remove(&result.path);
        self.active.set(self.active.get().saturating_sub(1));
        if self.retry.borrow_mut().remove(&result.path) {
            self.image(Some(&result.path));
            self.dispatch();
            return;
        }
        if let Some((width, height, bytes)) = result.pixels {
            let image = slint::Image::from_rgba8(
                slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(
                    &bytes, width, height,
                ),
            );
            let mut images = self.images.borrow_mut();
            let mut order = self.order.borrow_mut();
            images.insert(
                result.path.clone(),
                CachedCover {
                    image,
                    palette: result.palette,
                },
            );
            order.retain(|p| p != &result.path);
            order.push_back(result.path);
            while order.len() > 128 {
                if let Some(old) = order.pop_front() {
                    images.remove(&old);
                }
            }
        } else {
            if self.failed.borrow().len() >= 256 {
                self.failed.borrow_mut().clear();
            }
            self.failed.borrow_mut().insert(result.path);
        }
        self.dispatch();
    }
}
impl Drop for Covers {
    fn drop(&mut self) {
        self.cancelled
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let _ = self.requests.send(None);
        if let Some(worker) = self.worker.get_mut().take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests;
