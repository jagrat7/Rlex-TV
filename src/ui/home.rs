//! Home: a featured title across the top, then rows: what you were
//! watching (one click resumes), what the library just gained, and the
//! catalog rows every source offers. Every poster plays from its hover
//! button; clicking the poster opens its details.

use gpui_kit::assets::IconName;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme, StyledExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::common::{
    Load, TOP_BAR_HEIGHT, await_player, eid, error_text, fmt_runtime, fmt_time_left, hero_button,
    muted, short_episode,
};
use super::poster::{PosterCard, poster_row, skeleton_row};
use super::quickplay::{item_from_entry, play_title, play_video};
use super::ratings::RatingBadges;
use super::{DetailTarget, NavEvent};
use crate::addons::Meta;
use crate::state::{AppState, CatalogRef};
use crate::watch::WatchEntry;

/// How many of the first row's titles take turns being featured.
const FEATURED_POOL: usize = 8;
/// How far the rows ride up over the bottom of the billboard.
const ROWS_OVERLAP: f32 = 56.;

struct CatalogSlot {
    catalog: CatalogRef,
    items: Load<Vec<Meta>>,
}

pub struct HomeView {
    state: Entity<AppState>,
    rows: Vec<CatalogSlot>,
    /// Bumped whenever the catalog list changes, so late results for an old
    /// list are dropped.
    generation: usize,
    /// The title or video whose player is starting.
    starting: Option<String>,
    /// Which title of the pool the billboard shows; picked once per launch.
    featured_seed: usize,
}

impl EventEmitter<NavEvent> for HomeView {}

impl HomeView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |this, _, cx| {
            this.sync_catalogs(cx);
            cx.notify();
        })
        .detach();
        let mut this = Self {
            state,
            rows: Vec::new(),
            generation: 0,
            starting: None,
            featured_seed: crate::store::now() as usize,
        };
        this.sync_catalogs(cx);
        this
    }

    /// Reloads the rows when the set of catalogs changed (a source was added
    /// or removed); otherwise leaves what's loaded alone.
    fn sync_catalogs(&mut self, cx: &mut Context<Self>) {
        let catalogs = self.state.read(cx).home_catalogs();
        let same = catalogs.len() == self.rows.len()
            && catalogs
                .iter()
                .zip(&self.rows)
                .all(|(a, b)| catalog_key(a) == catalog_key(&b.catalog));
        if same {
            return;
        }
        self.generation += 1;
        self.rows = catalogs
            .into_iter()
            .map(|catalog| CatalogSlot {
                catalog,
                items: Load::Loading,
            })
            .collect();
        for index in 0..self.rows.len() {
            self.load_row(index, cx);
        }
    }

    fn load_row(&mut self, index: usize, cx: &mut Context<Self>) {
        let generation = self.generation;
        let catalog = self.rows[index].catalog.clone();
        let task = self
            .state
            .update(cx, |state, cx| state.load_catalog(&catalog, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                if let Some(slot) = this.rows.get_mut(index) {
                    slot.items = Load::from_result(result);
                    this.load_featured_ratings(cx);
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// The billboard's title: one with artwork from the first row that has
    /// titles. Rows are taken in order, waiting on one still loading, so the
    /// pick doesn't change as later rows arrive. `None` while it loads.
    fn featured(&self) -> Option<&Meta> {
        let items = self.rows.iter().find_map(|slot| match &slot.items {
            Load::Ready(items) if !items.is_empty() => Some(Some(items)),
            Load::Loading => Some(None),
            _ => None,
        })??;
        let pool: Vec<&Meta> = items
            .iter()
            .filter(|m| m.background.is_some())
            .take(FEATURED_POOL)
            .collect();
        if pool.is_empty() {
            return items.first();
        }
        Some(pool[self.featured_seed % pool.len()])
    }

    fn featured_loading(&self) -> bool {
        self.featured().is_none()
            && self
                .rows
                .iter()
                .any(|slot| matches!(slot.items, Load::Loading))
    }

    fn load_featured_ratings(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.featured().map(|m| m.id.clone()) else {
            return;
        };
        self.state
            .update(cx, |state, cx| state.load_ratings(&id, cx))
            .detach();
    }

    fn open(
        &self,
        target: DetailTarget,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |_, _, _, cx| cx.emit(NavEvent::OpenDetail(target.clone())))
    }

    fn play(
        &self,
        target: DetailTarget,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _, window, cx| {
            if this.starting.is_some() {
                super::common::still_starting(window, cx);
                return;
            }
            this.starting = Some(target.id.clone());
            cx.notify();
            let task = play_title(&this.state, target.clone(), cx);
            await_player(task, window, cx, |this: &mut Self| this.starting = None);
        })
    }

    fn resume(
        &self,
        entry: &WatchEntry,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        let item = item_from_entry(entry);
        cx.listener(move |this, _, window, cx| {
            if this.starting.is_some() {
                super::common::still_starting(window, cx);
                return;
            }
            this.starting = Some(item.video_id.clone());
            cx.notify();
            let task = play_video(&this.state, item.clone(), None, cx);
            await_player(task, window, cx, |this: &mut Self| this.starting = None);
        })
    }

    fn is_starting(&self, key: &str) -> bool {
        self.starting.as_deref() == Some(key)
    }

    // Rendering

    fn billboard(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let height = (window.viewport_size().height * 0.66).clamp(px(400.), px(620.));
        let theme = cx.theme();
        let bg = theme.background;
        if self.featured_loading() {
            return Some(
                div()
                    .relative()
                    .w_full()
                    .h(height)
                    .bg(theme.secondary)
                    .child(
                        v_flex()
                            .absolute()
                            .left_6()
                            .bottom(px(ROWS_OVERLAP + 40.))
                            .w(px(440.))
                            .gap_3()
                            .child(Skeleton::new().w(px(320.)).h(px(64.)))
                            .child(Skeleton::new().w(px(220.)).h_4())
                            .child(Skeleton::new().w_full().h_4())
                            .child(Skeleton::new().w_2_3().h_4()),
                    )
                    .into_any_element(),
            );
        }
        let meta = self.featured()?.clone();
        let target = DetailTarget::from_meta(&meta);
        let ratings = self.state.read(cx).ratings(&meta.id);
        let busy = self.is_starting(&meta.id);

        let mut facts: Vec<String> = Vec::new();
        facts.extend(meta.year_label().map(str::to_string));
        if meta.kind != "series" {
            facts.extend(meta.runtime.as_deref().map(fmt_runtime));
        }
        facts.push(
            if meta.kind == "series" {
                "Series"
            } else {
                "Movie"
            }
            .to_string(),
        );
        let genres = meta.genres().iter().take(3).cloned().collect::<Vec<_>>();

        let name: SharedString = meta.name.clone().into();
        let title_text = {
            let name = name.clone();
            move || {
                div()
                    .text_size(px(44.))
                    .line_height(px(50.))
                    .font_bold()
                    .line_clamp(2)
                    .text_ellipsis()
                    .child(name.clone())
                    .into_any_element()
            }
        };
        let title: AnyElement = match meta.logo.clone() {
            Some(logo) => div()
                .flex()
                .items_end()
                .child(
                    img(logo)
                        .h(px(110.))
                        .max_w(px(400.))
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(title_text.clone()),
                )
                .into_any_element(),
            None => title_text(),
        };

        let play_label = if busy { "Starting…" } else { "Play" };

        Some(
            div()
                .relative()
                .w_full()
                .h(height)
                .overflow_hidden()
                .bg(theme.secondary)
                .when_some(meta.background.clone(), |this, url| {
                    this.child(
                        img(url)
                            .absolute()
                            .inset_0()
                            .size_full()
                            .object_fit(ObjectFit::Cover),
                    )
                })
                // Darken the left for the text, and fade the bottom into the page.
                .child(div().absolute().inset_0().bg(linear_gradient(
                    90.,
                    linear_color_stop(bg.alpha(0.88), 0.),
                    linear_color_stop(bg.alpha(0.0), 0.65),
                )))
                .child(div().absolute().inset_0().bg(linear_gradient(
                    180.,
                    linear_color_stop(bg.alpha(0.0), 0.55),
                    linear_color_stop(bg, 1.),
                )))
                .child(
                    v_flex()
                        .absolute()
                        .left_6()
                        .bottom(px(ROWS_OVERLAP + 32.))
                        .w(relative(0.5))
                        .min_w(px(380.))
                        .max_w(px(520.))
                        .gap_3()
                        .child(title)
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .flex_wrap()
                                .children(
                                    Some(RatingBadges::from_ratings(
                                        ratings.as_ref(),
                                        meta.imdb_rating.as_deref(),
                                    ))
                                    .filter(|badges| !badges.is_empty()),
                                )
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.foreground.alpha(0.85))
                                        .child(facts.join("  ·  ")),
                                ),
                        )
                        .when(!genres.is_empty(), |this| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(genres.join("  •  ")),
                            )
                        })
                        .when_some(meta.description.clone(), |this, text| {
                            this.child(
                                div()
                                    .text_sm()
                                    .line_height(rems(1.45))
                                    .text_color(theme.foreground.alpha(0.9))
                                    .line_clamp(3)
                                    .text_ellipsis()
                                    .child(text),
                            )
                        })
                        .child(
                            h_flex()
                                .pt_2()
                                .gap_3()
                                .child(hero_button(
                                    "billboard-play",
                                    IconName::Play,
                                    play_label,
                                    true,
                                    self.play(target.clone(), cx),
                                ))
                                .child(hero_button(
                                    "billboard-info",
                                    IconName::Info,
                                    "More info",
                                    false,
                                    self.open(target, cx),
                                )),
                        ),
                )
                .into_any_element(),
        )
    }

    fn continue_watching(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let entries = self.state.read(cx).continue_watching();
        if entries.is_empty() {
            return None;
        }
        let cards: Vec<PosterCard> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                let left =
                    (entry.duration > 0.0).then(|| fmt_time_left(entry.position, entry.duration));
                let subtitle = match (&entry.episode_label, left) {
                    (Some(ep), Some(left)) => Some(format!("{} · {left}", short_episode(ep))),
                    (Some(ep), None) => Some(short_episode(ep)),
                    (None, left) => left,
                };
                PosterCard::new(eid(format!("continue-{i}")), entry.name.clone())
                    .poster(entry.poster.clone())
                    .subtitle(subtitle)
                    .progress(Some(entry.progress()))
                    .busy(self.is_starting(&entry.video_id))
                    .on_click(self.resume(entry, cx))
                    .on_info(self.open(DetailTarget::from_entry(entry), cx))
            })
            .collect();
        Some(row(
            "Continue watching",
            poster_row("continue-row", cards, window, cx).into_any_element(),
        ))
    }

    fn recently_added(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let titles = self.state.read(cx).recently_added(20);
        if titles.is_empty() {
            return None;
        }
        let cards: Vec<PosterCard> = titles
            .iter()
            .enumerate()
            .map(|(i, title)| {
                let target = DetailTarget::from_title(title);
                PosterCard::new(eid(format!("recent-{i}")), title.name.clone())
                    .poster(title.poster.clone())
                    .subtitle(title.year.clone())
                    .busy(self.is_starting(&title.meta_id))
                    .on_click(self.open(target.clone(), cx))
                    .on_play(self.play(target, cx))
            })
            .collect();
        Some(row(
            "New in your library",
            poster_row("recent-row", cards, window, cx).into_any_element(),
        ))
    }

    fn catalog_row(&self, index: usize, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let slot = &self.rows[index];
        let body: AnyElement = match &slot.items {
            Load::Loading => skeleton_row(cx).into_any_element(),
            Load::Failed(err) => {
                error_text(format!("Couldn't load this list: {err}"), cx).into_any_element()
            }
            Load::Ready(items) if items.is_empty() => {
                muted("Nothing here yet.", cx).into_any_element()
            }
            Load::Ready(items) => {
                let cards: Vec<PosterCard> = items
                    .iter()
                    .enumerate()
                    .map(|(i, meta)| {
                        let target = DetailTarget::from_meta(meta);
                        PosterCard::new(eid(format!("catalog-{index}-{i}")), meta.name.clone())
                            .poster(meta.poster.clone())
                            .subtitle(meta.year_label().map(str::to_string))
                            .rating(meta.imdb_rating.clone())
                            .busy(self.is_starting(&meta.id))
                            .on_click(self.open(target.clone(), cx))
                            .on_play(self.play(target, cx))
                    })
                    .collect();
                poster_row(
                    ElementId::NamedInteger("catalog".into(), index as u64),
                    cards,
                    window,
                    cx,
                )
                .into_any_element()
            }
        };
        row(slot.catalog.title.clone(), body)
    }
}

/// `S1 E2 · Pilot` → `S1 E2`, so the card's line stays short.
fn row(title: impl Into<SharedString>, body: AnyElement) -> Div {
    v_flex()
        .gap_2()
        .child(div().text_lg().font_semibold().child(title.into()))
        .child(body)
}

fn catalog_key(catalog: &CatalogRef) -> (&str, &str, &str) {
    (&catalog.base_url, &catalog.kind, &catalog.id)
}

impl Render for HomeView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let billboard = self.billboard(window, cx);
        let has_billboard = billboard.is_some();
        let continue_watching = self.continue_watching(window, cx);
        let recently_added = self.recently_added(window, cx);
        let rows: Vec<Div> = (0..self.rows.len())
            .map(|i| self.catalog_row(i, window, cx))
            .collect();
        v_flex()
            .id("home")
            .w_full()
            .pb_8()
            .children(billboard)
            .child(
                v_flex()
                    .relative()
                    .px_6()
                    .gap_6()
                    .map(|this| {
                        if has_billboard {
                            this.mt(px(-ROWS_OVERLAP))
                        } else {
                            this.pt(px(TOP_BAR_HEIGHT + 16.))
                        }
                    })
                    .when_some(continue_watching, |this, row| this.child(row))
                    .when_some(recently_added, |this, row| this.child(row))
                    .children(rows)
                    .when(self.rows.is_empty(), |this| {
                        this.child(muted("No catalogs to show.", cx))
                    }),
            )
    }
}
