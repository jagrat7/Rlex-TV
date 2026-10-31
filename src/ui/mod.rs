//! The GPUI user interface: a sidebar shell with Home, Search, Library,
//! Downloads and Settings pages, and a Detail page pushed on top of them.
//!
//! Every page holds the shared [`AppState`] entity, observes it so it
//! re-renders when the backend changes, and asks the shell to navigate by
//! emitting a [`NavEvent`].

mod common;
mod detail;
mod downloads;
mod home;
mod library;
mod poster;
mod search;
mod settings;
mod shell;

use std::sync::{Arc, OnceLock};

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::*;

use crate::addons::Meta;
use crate::library::LibraryTitle;
use crate::state::AppState;
use crate::watch::WatchEntry;

fn logo() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| {
        Arc::new(Image::from_bytes(
            ImageFormat::Png,
            include_bytes!("../../assets/logo/rlex-128.png").to_vec(),
        ))
    })
    .clone()
}

/// The app icon, baked into the binary. X11 shows it on the window; on
/// Wayland the compositor looks the icon up by `app_id` through the desktop
/// entry in `assets/linux/` instead.
fn window_icon() -> Option<Arc<image::RgbaImage>> {
    const PNG: &[u8] = include_bytes!("../../assets/logo/rlex-256.png");
    match image::load_from_memory_with_format(PNG, image::ImageFormat::Png) {
        Ok(img) => Some(Arc::new(img.into_rgba8())),
        Err(err) => {
            log::warn!("window icon won't show: {err:#}");
            None
        }
    }
}

pub fn open_main_window(state: Entity<AppState>, cx: &mut App) {
    Theme::change(ThemeMode::Dark, None, cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::centered(size(px(1280.), px(800.)), cx)),
        titlebar: Some(TitlebarOptions {
            title: Some("Rlex".into()),
            ..Default::default()
        }),
        window_min_size: Some(size(px(960.), px(600.))),
        app_id: Some("rlex".into()),
        icon: window_icon(),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| shell::Shell::new(state, window, cx))
    })
    .expect("failed to open the main window");
}

/// What a Detail page is about: enough to draw its header before the
/// metadata arrives, and to know whether there is metadata to fetch at all.
#[derive(Clone, Debug)]
pub struct DetailTarget {
    pub kind: String,
    pub id: String,
    pub name: String,
    pub poster: Option<String>,
}

impl DetailTarget {
    /// Titles that matched nothing online have no metadata to load.
    pub fn is_local(&self) -> bool {
        self.id.starts_with("local:")
    }

    pub fn from_meta(meta: &Meta) -> Self {
        Self {
            kind: meta.kind.clone(),
            id: meta.id.clone(),
            name: meta.name.clone(),
            poster: meta.poster.clone(),
        }
    }

    pub fn from_title(title: &LibraryTitle) -> Self {
        Self {
            kind: title.kind.clone(),
            id: title.meta_id.clone(),
            name: title.name.clone(),
            poster: title.poster.clone(),
        }
    }

    pub fn from_entry(entry: &WatchEntry) -> Self {
        Self {
            kind: entry.kind.clone(),
            id: entry.meta_id.clone(),
            name: entry.name.clone(),
            poster: entry.poster.clone(),
        }
    }
}

/// Pages emit this; the shell pushes the matching page.
pub enum NavEvent {
    OpenDetail(DetailTarget),
}
