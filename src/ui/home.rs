//! Home: what you were watching, what the library just gained, and the
//! catalog rows every source offers.

use gpui_kit::component::{ActiveTheme, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::common::{Load, eid, error_text, muted, page, page_title, section};
use super::poster::{PosterCard, poster_row, skeleton_row};
use super::{DetailTarget, NavEvent};
use crate::addons::Meta;
use crate::state::{AppState, CatalogRef};

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
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn open(
        &self,
        target: DetailTarget,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |_, _, _, cx| cx.emit(NavEvent::OpenDetail(target.clone())))
    }

    fn continue_watching(&self, cx: &mut Context<Self>) -> Option<Div> {
        let entries = self.state.read(cx).continue_watching();
        if entries.is_empty() {
            return None;
        }
        let cards: Vec<PosterCard> = entries
            .iter()
            .enumerate()
            .map(|(i, entry)| {
                PosterCard::new(eid(format!("continue-{i}")), entry.name.clone())
                    .poster(entry.poster.clone())
                    .subtitle(entry.episode_label.clone())
                    .progress(Some(entry.progress()))
                    .on_click(self.open(DetailTarget::from_entry(entry), cx))
            })
            .collect();
        Some(section(
            "Continue watching",
            poster_row("continue-row", cards),
        ))
    }

    fn recently_added(&self, cx: &mut Context<Self>) -> Option<Div> {
        let titles = self.state.read(cx).recently_added(20);
        if titles.is_empty() {
            return None;
        }
        let cards: Vec<PosterCard> = titles
            .iter()
            .enumerate()
            .map(|(i, title)| {
                PosterCard::new(eid(format!("recent-{i}")), title.name.clone())
                    .poster(title.poster.clone())
                    .subtitle(title.year.clone())
                    .on_click(self.open(DetailTarget::from_title(title), cx))
            })
            .collect();
        Some(section("Recently added", poster_row("recent-row", cards)))
    }

    fn catalog_row(&self, index: usize, cx: &mut Context<Self>) -> Div {
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
                        PosterCard::new(eid(format!("catalog-{index}-{i}")), meta.name.clone())
                            .poster(meta.poster.clone())
                            .subtitle(meta.year_label().map(str::to_string))
                            .on_click(self.open(DetailTarget::from_meta(meta), cx))
                    })
                    .collect();
                poster_row(row_id(index), cards).into_any_element()
            }
        };
        section(slot.catalog.title.clone(), body)
    }
}

fn catalog_key(catalog: &CatalogRef) -> (&str, &str, &str) {
    (&catalog.base_url, &catalog.kind, &catalog.id)
}

/// Scroll state is keyed by a static id, so each row gets one from a table.
fn row_id(index: usize) -> &'static str {
    const IDS: [&str; 12] = [
        "catalog-0",
        "catalog-1",
        "catalog-2",
        "catalog-3",
        "catalog-4",
        "catalog-5",
        "catalog-6",
        "catalog-7",
        "catalog-8",
        "catalog-9",
        "catalog-10",
        "catalog-11",
    ];
    IDS[index % IDS.len()]
}

impl Render for HomeView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let continue_watching = self.continue_watching(cx);
        let recently_added = self.recently_added(cx);
        let rows: Vec<Div> = (0..self.rows.len())
            .map(|i| self.catalog_row(i, cx))
            .collect();
        page("home")
            .child(page_title("Home"))
            .when_some(continue_watching, |this, row| this.child(row))
            .when_some(recently_added, |this, row| this.child(row))
            .children(rows)
            .when(self.rows.is_empty(), |this| {
                this.child(
                    v_flex()
                        .text_color(cx.theme().muted_foreground)
                        .child("No catalogs to show."),
                )
            })
    }
}
