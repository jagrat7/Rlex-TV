//! Search results, grouped by the catalog that answered. The input lives
//! in the shell's top bar; this view owns its state and runs the searches.

use gpui_kit::assets::IconName;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::*;

use super::common::{
    DEBOUNCE, Load, eid, empty_state, error_text, muted, page, page_title, section,
};
use super::poster::{PosterCard, poster_row, skeleton_row};
use super::quickplay::play_title;
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
    /// The title whose player is starting from a card's Play button.
    starting: Option<String>,
    _subscription: Subscription,
}

impl EventEmitter<NavEvent> for SearchView {}

impl SearchView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Titles, people, genres")
                .clean_on_escape()
        });
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
            starting: None,
            _subscription: subscription,
        }
    }

    /// The search box, for the top bar.
    pub fn input(&self) -> Entity<InputState> {
        self.input.clone()
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

    fn play(&mut self, target: DetailTarget, window: &mut Window, cx: &mut Context<Self>) {
        if self.starting.is_some() {
            super::common::still_starting(window, cx);
            return;
        }
        self.starting = Some(target.id.clone());
        cx.notify();
        let task = play_title(&self.state, target, cx);
        super::common::await_player(task, window, cx, |this: &mut Self| this.starting = None);
    }

    fn rows(&self, rows: &[CatalogRow], window: &mut Window, cx: &mut Context<Self>) -> Vec<Div> {
        rows.iter()
            .enumerate()
            .map(|(r, row)| {
                let cards: Vec<PosterCard> = row
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, meta)| {
                        let target = DetailTarget::from_meta(meta);
                        let play = target.clone();
                        let busy = self.starting.as_deref() == Some(target.id.as_str());
                        PosterCard::new(eid(format!("search-{r}-{i}")), meta.name.clone())
                            .poster(meta.poster.clone())
                            .subtitle(meta.year_label().map(str::to_string))
                            .rating(meta.imdb_rating.clone())
                            .busy(busy)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(NavEvent::OpenDetail(target.clone()))
                            }))
                            .on_play(cx.listener(move |this, _, window, cx| {
                                this.play(play.clone(), window, cx)
                            }))
                    })
                    .collect();
                section(
                    row.title.clone(),
                    poster_row(
                        ElementId::NamedInteger("search".into(), r as u64),
                        cards,
                        window,
                        cx,
                    ),
                )
            })
            .collect()
    }
}

impl Render for SearchView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                .rows(rows, window, cx)
                .into_iter()
                .map(IntoElement::into_any_element)
                .collect(),
            Results::Done(_, Load::Loading) => vec![skeleton_row(cx).into_any_element()],
        };

        let heading = match &self.results {
            Results::Done(query, _) => format!("Results for “{query}”"),
            _ => "Search".to_string(),
        };
        page("search").child(page_title(heading)).children(results)
    }
}
