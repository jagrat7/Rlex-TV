//! Home: a featured title across the top, then shelves: what you were
//! watching (one click resumes), what the library just gained, and the
//! catalog rows every source offers. Every poster plays from its hover
//! button; clicking the poster opens its details.

use std::f32::consts::{FRAC_PI_2, PI};

use rune::{Cx, El, SurfaceFit, col, el, row, text};

use super::format::{fmt_runtime, fmt_time_left, short_episode};
use super::play::item_from_entry;
use super::poster::{Card, POSTER_H, skeleton_strip};
use super::ratings::badges;
use super::strip::strip;
use super::theme::{space, t, ty};
use super::topbar::BAR_H;
use super::widgets::{Button, Size, Variant, cover, dotted, error_text, muted, skeleton};
use super::{Action, App, DetailTarget, Load, Page, icons, on, scroller};
use crate::addons::Meta;
use crate::state::CatalogRef;

/// How many of the first row's titles take turns being featured.
const FEATURED_POOL: usize = 8;
/// How far the shelves ride up over the bottom of the billboard.
const OVERLAP: f32 = 72.0;

pub struct Home {
    pub rows: Vec<Shelf>,
    /// Bumped whenever the catalog list changes, so late results for an old
    /// list are dropped.
    generation: u64,
    /// Which title of the pool the billboard shows; picked once per launch.
    seed: usize,
}

impl Default for Home {
    fn default() -> Self {
        // Headless renders feature the same title every time.
        let seed = if rune::is_headless() {
            0
        } else {
            crate::store::now() as usize
        };
        Self {
            rows: Vec::new(),
            generation: 0,
            seed,
        }
    }
}

pub struct Shelf {
    pub catalog: CatalogRef,
    pub items: Load<Vec<Meta>>,
}

impl Home {
    /// The billboard's title: one with artwork from the first row that has
    /// titles. Rows are taken in order, waiting on one still loading, so the
    /// pick doesn't change as later rows arrive. `None` while it loads.
    fn featured(&self) -> Option<&Meta> {
        let items = self.rows.iter().find_map(|row| match &row.items {
            Load::Ready(items) if !items.is_empty() => Some(Some(items)),
            Load::Loading => Some(None),
            _ => None,
        })??;
        let pool: Vec<&Meta> = items
            .iter()
            .filter(|m| m.background.is_some())
            .take(FEATURED_POOL)
            .collect();
        match pool.len() {
            0 => items.first(),
            n => Some(pool[self.seed % n]),
        }
    }

    fn featured_loading(&self) -> bool {
        self.featured().is_none()
            && self
                .rows
                .iter()
                .any(|row| matches!(row.items, Load::Loading))
    }
}

impl App {
    /// Reloads the shelves when the set of catalogs changed (a source was
    /// added or removed); otherwise leaves what's loaded alone.
    pub fn sync_home(&mut self) {
        let catalogs = self.core.home_catalogs();
        if catalogs
            .iter()
            .eq(self.home.rows.iter().map(|row| &row.catalog))
        {
            return;
        }
        self.home.generation += 1;
        let generation = self.home.generation;
        self.home.rows = catalogs
            .into_iter()
            .map(|catalog| Shelf {
                catalog,
                items: Load::Loading,
            })
            .collect();
        for (i, row) in self.home.rows.iter().enumerate() {
            self.bg
                .run(self.core.catalog(&row.catalog), move |s, items, _| {
                    if s.home.generation != generation {
                        return;
                    }
                    if let Some(row) = s.home.rows.get_mut(i) {
                        row.items = Load::from_result(items);
                    }
                    if let Some(id) = s.home.featured().map(|m| m.id.clone()) {
                        s.load_ratings(&id);
                    }
                });
        }
    }
}

pub fn page(s: &App, cx: &mut Cx<App>) -> El {
    let [w, h] = cx.layout_size("app").unwrap_or([1280.0, 800.0]);
    let scale = cx.env().scale;
    let billboard = billboard(s, w, (h * 0.68).clamp(440.0, 660.0));
    let has_billboard = billboard.is_some();

    let mut shelves = col().key("shelves").w_full().gap(space::SECTION).pb(64.0);
    shelves = if has_billboard {
        shelves.mt(-OVERLAP)
    } else {
        shelves.pt(BAR_H + 24.0)
    };
    shelves = shelves
        .children(continue_watching(s, scale))
        .children(recently_added(s, scale));
    for (i, row) in s.home.rows.iter().enumerate() {
        let body = match &row.items {
            Load::Loading => skeleton_strip(),
            Load::Failed(err) => {
                error_text(format!("Couldn't load this list: {err}")).mx(space::PAGE)
            }
            Load::Ready(items) if items.is_empty() => muted("Nothing here yet.").mx(space::PAGE),
            Load::Ready(items) => meta_strip(s, &format!("home-{i}"), items, scale),
        };
        shelves = shelves.child(shelf(&row.catalog.title, body));
    }
    scroller(
        s,
        Page::Home,
        col()
            .key("home")
            .w_full()
            .children(billboard)
            .child(shelves),
    )
}

/// A titled shelf: the heading in the page margin, the strip edge to edge.
pub fn shelf(title: &str, body: El) -> El {
    col()
        .key(title)
        .w_full()
        .gap(12.0)
        .child(
            text(title)
                .font(ty::HEADING)
                .color(t().text)
                .px(space::PAGE),
        )
        .child(body)
}

/// A strip of catalog titles: clicking opens one, its Play plays it.
pub fn meta_strip(s: &App, key: &str, items: &[Meta], scale: f32) -> El {
    strip(s, key, items.len(), POSTER_H, |i| {
        meta_card(s, i, &items[i], scale)
    })
}

pub fn meta_card(s: &App, i: usize, meta: &Meta, scale: f32) -> El {
    let target = DetailTarget::from_meta(meta);
    Card::new(format!("{i}"), meta.name.clone())
        .poster(meta.poster.as_deref())
        .subtitle(meta.year_label().map(str::to_string))
        .rating(meta.imdb_rating.as_deref())
        .busy(s.is_starting(&meta.id))
        .on_open(Action::Open(target.clone()))
        .on_play(Action::PlayTitle(target))
        .build(s, scale)
}

fn continue_watching(s: &App, scale: f32) -> Option<El> {
    let entries = s.core.watch.continue_watching();
    if entries.is_empty() {
        return None;
    }
    let cards = strip(s, "home-continue", entries.len(), POSTER_H, |i| {
        let entry = &entries[i];
        let left = (entry.duration > 0.0).then(|| fmt_time_left(entry.position, entry.duration));
        Card::new(format!("{i}"), entry.name.clone())
            .poster(entry.poster.as_deref())
            .subtitle(Some(dotted([
                entry.episode_label.as_deref().map(short_episode),
                left,
            ])))
            .progress(Some(entry.progress()))
            .busy(s.is_starting(&entry.video_id))
            .on_open(Action::PlayVideo(item_from_entry(entry)))
            .on_info(Action::Open(DetailTarget::from_entry(entry)))
            .build(s, scale)
    });
    Some(shelf("Continue watching", cards))
}

fn recently_added(s: &App, scale: f32) -> Option<El> {
    let titles = s.core.library.recently_added(20);
    if titles.is_empty() {
        return None;
    }
    let cards = strip(s, "home-recent", titles.len(), POSTER_H, |i| {
        let title = &titles[i];
        let target = DetailTarget::from_title(title);
        Card::new(format!("{i}"), title.name.clone())
            .poster(title.poster.as_deref())
            .subtitle(title.year.clone())
            .busy(s.is_starting(&title.meta_id))
            .on_open(Action::Open(target.clone()))
            .on_play(Action::PlayTitle(target))
            .build(s, scale)
    });
    Some(shelf("New in your library", cards))
}

/// The featured title across the top: its backdrop, logo, scores, story,
/// and Play.
fn billboard(s: &App, width: f32, height: f32) -> Option<El> {
    let th = t();
    let stage = el().key("billboard").w_full().h(height).fill(th.raised);
    let text_box = |e: El| {
        e.pin(Some(space::PAGE), None, None, Some(OVERLAP + 40.0))
            .w((width * 0.46).clamp(400.0, 560.0))
    };
    if s.home.featured_loading() {
        return Some(
            stage.child(text_box(
                col()
                    .gap(14.0)
                    .child(skeleton(Some(340.0), 76.0))
                    .child(skeleton(Some(220.0), 16.0))
                    .child(skeleton(None, 14.0))
                    .child(skeleton(Some(300.0), 14.0))
                    .child(
                        row()
                            .gap(12.0)
                            .pt(8.0)
                            .child(skeleton(Some(130.0), 46.0))
                            .child(skeleton(Some(150.0), 46.0)),
                    ),
            )),
        );
    }
    let meta = s.home.featured()?;
    let target = DetailTarget::from_meta(meta);
    let series = meta.kind == "series";
    let facts = dotted([
        meta.year_label().map(str::to_string),
        (!series)
            .then(|| meta.runtime.as_deref().map(fmt_runtime))
            .flatten(),
        Some(if series { "Series" } else { "Movie" }.to_string()),
    ]);

    let mut info = col()
        .gap(14.0)
        .child(title_art(s, meta, 420.0, 120.0, ty::DISPLAY));
    info = info.child(
        row()
            .gap(16.0)
            .center_y()
            .children(badges(
                s.core.ratings(&meta.id).as_ref(),
                meta.imdb_rating.as_deref(),
                false,
            ))
            .child(
                text(facts)
                    .font(ty::SMALL_STRONG)
                    .color(th.text.alpha(0.88)),
            ),
    );
    let genres = meta.genres().iter().take(3).cloned().collect::<Vec<_>>();
    if !genres.is_empty() {
        info = info.child(text(genres.join("  •  ")).font(ty::SMALL).color(th.muted));
    }
    if let Some(about) = &meta.description {
        info = info.child(
            text(about.clone())
                .font(ty::BODY)
                .color(th.text.alpha(0.9))
                .max_lines(3),
        );
    }
    info = info.child(
        row()
            .gap(12.0)
            .pt(8.0)
            .child(
                Button::new(if s.is_starting(&meta.id) {
                    "Starting…"
                } else {
                    "Play"
                })
                .with_icon(&icons::PLAY)
                .variant(Variant::Light)
                .size(Size::Large)
                .busy(s.is_starting(&meta.id))
                .key("billboard-play")
                .on_press(on(Action::PlayTitle(target.clone())))
                .build(),
            )
            .child(
                Button::new("More info")
                    .with_icon(&icons::INFO)
                    .variant(Variant::Glass)
                    .size(Size::Large)
                    .key("billboard-info")
                    .on_press(on(Action::Open(target)))
                    .build(),
            ),
    );

    let mut stage = stage;
    if let Some(url) = &meta.background {
        stage = stage.child(cover().pixels_fit(&s.images.get(url, None).pixels, SurfaceFit::Fill));
    }
    Some(stage.child(backdrop_shade(th.bg)).child(text_box(info)))
}

/// Darkens artwork on the left for the text, and fades its bottom into the
/// page.
pub fn backdrop_shade(bg: rune::Color) -> El {
    cover()
        .child(cover().linear_gradient(
            FRAC_PI_2,
            [
                (0.0, bg.alpha(0.92)),
                (0.38, bg.alpha(0.55)),
                (0.7, bg.alpha(0.0)),
            ],
        ))
        .child(
            cover().linear_gradient(PI, [(0.5, bg.alpha(0.0)), (0.85, bg.alpha(0.7)), (1.0, bg)]),
        )
}

/// A title's logo art, or its name in big type when it has none (or it
/// didn't load).
pub fn title_art(s: &App, meta: &Meta, w: f32, h: f32, font: rune::TextStyle) -> El {
    let name = || {
        text(meta.name.clone())
            .font(font)
            .color(t().text)
            .max_lines(2)
    };
    match meta.logo.as_deref().map(|url| s.images.get(url, None)) {
        // Only the height is set: the logo's own shape gives the width, so
        // it sits flush left however wide it is.
        Some(art) if !art.failed => el()
            .key("logo")
            .h(h)
            .max_w(w)
            .self_start()
            .pixels_fit(&art.pixels, SurfaceFit::Fit)
            .label(meta.name.clone()),
        _ => name(),
    }
}
