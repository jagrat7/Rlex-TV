//! Artwork from the web: posters, backdrops, logos and episode stills.
//!
//! The view asks for a URL and gets a [`Pixels`] handle at once. The image
//! is fetched and decoded on Tokio, scaled down to the size it's drawn at,
//! and set into that handle, which redraws only the windows showing it: an
//! arriving poster never runs the view. A failure does, so the card can
//! show its title instead.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use image::imageops::FilterType;
use rune::Pixels;
use tokio::sync::Semaphore;

use super::Bg;

/// Decoded pixels kept for images not on screen, before the oldest go.
const BUDGET_BYTES: usize = 320 << 20;
/// Downloads at a time; the rest wait their turn.
const PARALLEL: usize = 8;

/// What the view draws: the pixels (empty until they arrive) and whether
/// they never will.
pub struct Art {
    pub pixels: Pixels,
    pub failed: bool,
}

pub struct Images {
    bg: Bg,
    http: reqwest::Client,
    limit: Arc<Semaphore>,
    entries: RefCell<HashMap<(String, [u32; 2]), Entry>>,
    /// The app's logo, decoded at launch.
    pub logo: Pixels,
}

struct Entry {
    pixels: Pixels,
    failed: Arc<AtomicBool>,
    bytes: Arc<AtomicUsize>,
    /// The commit that last drew it.
    used: Cell<u64>,
}

impl Images {
    pub fn new(bg: Bg) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Rlex/", env!("CARGO_PKG_VERSION")))
            .build()
            .unwrap_or_default();
        let logo = Pixels::new();
        match decode(include_bytes!("../../assets/logo/rlex-128.png"), None) {
            Ok((w, h, rgba)) => logo.set(w, h, rgba),
            Err(err) => log::warn!("logo: {err:#}"),
        }
        Self {
            bg,
            http,
            limit: Arc::new(Semaphore::new(PARALLEL)),
            entries: RefCell::default(),
            logo,
        }
    }

    /// The image at `url`, scaled to cover `px` device pixels (or left at its
    /// own size with `None`, for art drawn large). Starts the download the
    /// first time it's asked for.
    pub fn get(&self, url: &str, px: Option<[u32; 2]>) -> Art {
        let key = (url.to_string(), px.unwrap_or_default());
        let mut entries = self.entries.borrow_mut();
        let entry = entries.entry(key).or_insert_with(|| self.fetch(url, px));
        entry.used.set(rune::commit_generation());
        Art {
            pixels: entry.pixels.clone(),
            failed: entry.failed.load(Ordering::Relaxed),
        }
    }

    fn fetch(&self, url: &str, px: Option<[u32; 2]>) -> Entry {
        let entry = Entry {
            pixels: Pixels::new(),
            failed: Arc::default(),
            bytes: Arc::default(),
            used: Cell::new(0),
        };
        let (pixels, failed, bytes) = (
            entry.pixels.clone(),
            entry.failed.clone(),
            entry.bytes.clone(),
        );
        let (http, limit, bg, url) = (
            self.http.clone(),
            self.limit.clone(),
            self.bg.clone(),
            url.to_string(),
        );
        self.bg.spawn(async move {
            let result = async {
                let _turn = limit.acquire().await?;
                let body = http
                    .get(&url)
                    .send()
                    .await?
                    .error_for_status()?
                    .bytes()
                    .await?;
                tokio::task::spawn_blocking(move || decode(&body, px)).await?
            }
            .await;
            match result {
                Ok((w, h, rgba)) => {
                    bytes.store(rgba.len(), Ordering::Relaxed);
                    pixels.set(w, h, rgba);
                }
                Err(err) => {
                    log::debug!("image {url}: {err:#}");
                    failed.store(true, Ordering::Relaxed);
                    // The view shows a stand-in instead.
                    bg.update(|_, _| {});
                }
            }
        });
        entry
    }

    /// Forgets the least recently drawn images once they hold more than the
    /// budget. Those drawn by the last couple of commits always stay.
    pub fn trim(&self) {
        let mut entries = self.entries.borrow_mut();
        let size = |e: &Entry| e.bytes.load(Ordering::Relaxed);
        let mut total: usize = entries.values().map(size).sum();
        if total <= BUDGET_BYTES {
            return;
        }
        let recent = rune::commit_generation().saturating_sub(2);
        let mut old: Vec<_> = entries
            .iter()
            .filter(|(_, e)| e.used.get() < recent)
            .map(|(k, e)| (e.used.get(), k.clone()))
            .collect();
        old.sort();
        for (_, key) in old {
            if total <= BUDGET_BYTES * 3 / 4 {
                break;
            }
            if let Some(entry) = entries.remove(&key) {
                total -= size(&entry);
            }
        }
    }
}

/// Decodes an image, scaled down and cropped to cover `px` when it's larger.
fn decode(bytes: &[u8], px: Option<[u32; 2]>) -> anyhow::Result<(u32, u32, Vec<u8>)> {
    let mut img = image::load_from_memory(bytes)?;
    if let Some([w, h]) = px
        && w > 0
        && h > 0
        && (img.width() > w || img.height() > h)
    {
        img = img.resize_to_fill(w, h, FilterType::CatmullRom);
    }
    let mut rgba = img.into_rgba8();
    // Logos come with clear margins; without them the art sits where its
    // box says, flush left beside the text.
    if let Some((x, y, w, h)) = opaque_bounds(&rgba)
        && (w, h) != rgba.dimensions()
    {
        rgba = image::imageops::crop_imm(&rgba, x, y, w, h).to_image();
    }
    Ok((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// The smallest box holding every pixel that isn't (nearly) clear.
fn opaque_bounds(img: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0, 0);
    for (x, y, px) in img.enumerate_pixels() {
        if px[3] > 8 {
            (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x), y1.max(y));
        }
    }
    (x0 <= x1).then(|| (x0, y0, x1 - x0 + 1, y1 - y0 + 1))
}

/// Device pixels for a box of `size` points.
pub fn px(size: [f32; 2], scale: f32) -> Option<[u32; 2]> {
    Some(size.map(|v| (v * scale).ceil() as u32))
}
