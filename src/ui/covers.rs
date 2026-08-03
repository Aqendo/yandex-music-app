//! Cover-art loading for the UI.
//!
//! Covers are requested lazily: `image()` only registers an `Image` widget for
//! a cover URL, and the UI asks for downloads via `request()` (or
//! `request_image()`) once that widget is actually visible. Bytes are fetched
//! off the main thread (through the worker), decoded into `gdk::Texture` on the
//! main thread and cached, so revisiting a list never re-downloads.
use std::cell::RefCell;
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use gtk4::glib;
use gtk4::Image;

use crate::state::WorkerCommand;
use crate::worker::Worker;

/// Maximum number of decoded cover textures retained in memory.
const MAX_CACHE_ENTRIES: usize = 250;

/// Downloads cover thumbnails on demand, caches the decoded textures and
/// applies them to any number of `Image` widgets.
#[derive(Clone)]
pub struct Covers {
    cache: Rc<RefCell<HashMap<String, gtk4::gdk::Texture>>>,
    order: Rc<RefCell<VecDeque<String>>>,
    pending: Rc<RefCell<HashMap<String, Vec<Image>>>>,
    image_url: Rc<RefCell<HashMap<Image, String>>>,
    requested: Rc<RefCell<HashSet<String>>>,
    worker: Worker,
}

impl Covers {
    pub fn new(worker: Worker) -> Self {
        Self {
            cache: Rc::new(RefCell::new(HashMap::new())),
            order: Rc::new(RefCell::new(VecDeque::new())),
            pending: Rc::new(RefCell::new(HashMap::new())),
            image_url: Rc::new(RefCell::new(HashMap::new())),
            requested: Rc::new(RefCell::new(HashSet::new())),
            worker,
        }
    }

    /// Apply `url`'s cover to a reused `image`, downloading it if needed.
    pub fn apply(&self, image: &Image, url: Option<String>) {
        let Some(url) = url else { return };
        if let Some(texture) = self.cache.borrow().get(&url).cloned() {
            image.set_paintable(Some(&texture));
            return;
        }
        self.image_url.borrow_mut().insert(image.clone(), url.clone());
        self.pending
            .borrow_mut()
            .entry(url)
            .or_default()
            .push(image.clone());
    }

    /// Create a fresh image showing `url`'s cover at `pixel_size`.
    pub fn image(&self, url: Option<String>, pixel_size: i32) -> Image {
        let image = Image::new();
        image.set_pixel_size(pixel_size);
        self.apply(&image, url);
        image
    }

    /// Ask for `url` to be downloaded once (idempotent). No-op if already
    /// cached or already requested.
    pub fn request(&self, url: &str) {
        if self.cache.borrow().contains_key(url) {
            return;
        }
        if self.requested.borrow_mut().insert(url.to_string()) {
            self.worker
                .send(WorkerCommand::FetchCover { url: url.to_string() });
        }
    }

    /// Ask for the cover behind `image` to be downloaded (idempotent).
    pub fn request_image(&self, image: &Image) {
        if let Some(url) = self.image_url.borrow().get(image).cloned() {
            self.request(&url);
        }
    }

    /// Deliver downloaded bytes (main thread); fills any pending images.
    pub fn on_ready(&self, url: &str, bytes: Vec<u8>) {
        let Ok(texture) = gtk4::gdk::Texture::from_bytes(&glib::Bytes::from_owned(bytes)) else {
            return;
        };
        let mut image_url = self.image_url.borrow_mut();
        if let Some(pending) = self.pending.borrow_mut().remove(url) {
            for image in pending {
                image.set_paintable(Some(&texture));
                image_url.remove(&image);
            }
        }
        
        let mut cache = self.cache.borrow_mut();
        let mut order = self.order.borrow_mut();
        if !cache.contains_key(url) {
            order.push_back(url.to_string());
            cache.insert(url.to_string(), texture);
            if order.len() > MAX_CACHE_ENTRIES {
                if let Some(oldest) = order.pop_front() {
                    cache.remove(&oldest);
                    self.requested.borrow_mut().remove(&oldest);
                }
            }
        }
    }
}
