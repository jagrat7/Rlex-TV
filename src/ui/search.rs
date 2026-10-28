//! Search: one input, results grouped by the catalog that answered.

use gpui_kit::assets::IconName;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::*;

use super::common::{
    DEBOUNCE, Load, eid, empty_state, error_text, muted, page, page_title, section,
};
use super::poster::{PosterCard, poster_row, skeleton_row};
use super::{DetailTarget, NavEvent};
use crate::state::{AppState, CatalogRow};

enum Results {
    Idle,
    Loading,
    Done(String, Load<Vec<CatalogRow>>),
}

pub struct SearchView {
    state: Entity<AppState>,
    input: Entity<InputState>,
    results: Results,
    /// Bumped on every keystroke and every search, so a stale debounce or a
    /// slow earlier search can't overwrite a newer one.
    generation: usize,
    _subscription: Subscription,
}

impl EventEmitter<NavEvent> for SearchView {}

impl SearchView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search movies and series"));
        let subscription = cx.subscribe(&input, |this, _, event: &InputEvent, cx| match event {
            InputEvent::PressEnter { .. } => this.search_now(cx),
            InputEvent::Change => this.search_soon(cx),
            _ => {}
        });
        Self {
            state,
            input,
            results: Results::Idle,
            generation: 0,
            _subscription: subscription,
        }
    }

    /// Fills the box and searches straight away.
    #[cfg(debug_assertions)]
    pub fn set_query(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(query, window, cx));
        self.search_now(cx);
    }

    fn search_soon(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.search_now(cx);
                }
            })
            .ok();
        })
        .detach();
    }

    fn search_now(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let query = self.input.read(cx).value().trim().to_string();
        if query.is_empty() {
            self.results = Results::Idle;
            cx.notify();
            return;
        }
        self.results = Results::Loading;
        cx.notify();
        let task = self.state.update(cx, |state, cx| state.search(&query, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.generation == generation {
                    this.results = Results::Done(query, Load::from_result(result));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    fn rows(&self, rows: &[CatalogRow], cx: &mut Context<Self>) -> Vec<Div> {
        rows.iter()
            .enumerate()
            .map(|(r, row)| {
                let cards: Vec<PosterCard> = row
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, meta)| {
                        let target = DetailTarget::from_meta(meta);
                        PosterCard::new(eid(format!("search-{r}-{i}")), meta.name.clone())
                            .poster(meta.poster.clone())
                            .subtitle(meta.year_label().map(str::to_string))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(NavEvent::OpenDetail(target.clone()))
                            }))
                    })
                    .collect();
                section(row.title.clone(), poster_row(row_id(r), cards))
            })
            .collect()
    }
}

fn row_id(index: usize) -> &'static str {
    const IDS: [&str; 8] = [
        "search-0", "search-1", "search-2", "search-3", "search-4", "search-5", "search-6",
        "search-7",
    ];
    IDS[index % IDS.len()]
}

impl Render for SearchView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let results: Vec<AnyElement> = match &self.results {
            Results::Idle => vec![
                muted(
                    "Type to search Cinemeta and any catalogs your sources offer.",
                    cx,
                )
                .into_any_element(),
            ],
            Results::Loading => vec![skeleton_row(cx).into_any_element()],
            Results::Done(_, Load::Failed(err)) => {
                vec![error_text(format!("Search failed: {err}"), cx).into_any_element()]
            }
            Results::Done(query, Load::Ready(rows)) if rows.is_empty() => vec![
                empty_state(
                    IconName::SearchX,
                    format!("No results for “{query}”"),
                    "Try another spelling, or a shorter title.",
                    cx,
                )
                .into_any_element(),
            ],
            Results::Done(_, Load::Ready(rows)) => self
                .rows(rows, cx)
                .into_iter()
                .map(IntoElement::into_any_element)
                .collect(),
            Results::Done(_, Load::Loading) => vec![skeleton_row(cx).into_any_element()],
        };

        page("search")
            .child(page_title("Search"))
            .child(
                div().max_w(px(640.)).child(
                    Input::new(&self.input)
                        .prefix(Icon::new(IconName::Search))
                        .cleanable(true),
                ),
            )
            .children(results)
    }
}
