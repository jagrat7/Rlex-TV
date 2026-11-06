//! Named scenes, so every page can be rendered and checked headlessly:
//!
//! ```sh
//! cargo run -- --snap home.png --scene home --at 1
//! cargo run -- --tree detail.json --scene series --at 1
//! cargo run -- --list-scenes
//! ```
//!
//! A headless run reads the real library and history but never saves them,
//! and waits in `poll` for network answers, so what it shows is what a
//! window would a moment after launch.

use super::{App, DetailTarget, Page};

/// A scene: its name, and what it does to the state a launch starts with.
type Scene = (&'static str, fn(&mut App));

pub const SCENES: &[Scene] = &[
    ("home", |_| {}),
    ("library", |s| s.nav.page = Page::Library),
    ("downloads", |s| s.nav.page = Page::Downloads),
    ("settings", |s| s.nav.page = Page::Settings),
    ("search", |s| {
        let rows = crate::runtime::get().block_on(s.core.search("star wars"));
        s.nav.page = Page::Search;
        s.search.show("star wars", rows);
    }),
    ("movie", |s| {
        s.open_detail(target("movie", "tt1375666", "Inception"))
    }),
    ("series", |s| {
        s.open_detail(target("series", "tt0903747", "Breaking Bad"));
        if let Some(d) = s.nav.details.last_mut() {
            d.preselect = Some("tt0903747:1:2".into());
        }
    }),
    ("stacked", |s| {
        s.open_detail(target("movie", "tt1375666", "Inception"));
        s.open_detail(target("movie", "tt0816692", "Interstellar"));
    }),
    ("remove", |s| {
        s.nav.page = Page::Downloads;
        s.downloads.confirm = Some((0, "Big Buck Bunny".into()));
    }),
    ("toasts", |s| {
        s.toast_success("Added Example Source");
        s.toast_error("Couldn't play: No source has a stream for this yet.");
    }),
];

fn target(kind: &str, id: &str, name: &str) -> DetailTarget {
    DetailTarget {
        kind: kind.into(),
        id: id.into(),
        name: name.into(),
        poster: None,
    }
}
