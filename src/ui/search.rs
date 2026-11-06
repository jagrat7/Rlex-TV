//! Search: the box in the top bar, and results grouped by the catalog that
//! answered. Typing anywhere starts a search; emptying the box goes back.
//!
//! Requests go through Rune's `Latest`: each keystroke supersedes the last,
//! after a short pause, and only the newest answer lands.

use std::time::Duration;

use rune::{Cx, El, Latest, TextEdit, Tx, Waker, col};

use super::poster::skeleton_strip;
use super::theme::space;
use super::topbar::BAR_H;
use super::widgets::{empty_state, error_text, muted, page_title};
use super::{App, Load, Page, home, icons, scroller};
use crate::state::{CatalogRow, Core};

/// The search field's key, to tell when it has the keyboard.
pub const FIELD: &str = "search-field";

type Answer = (String, Result<Vec<CatalogRow>, String>);

pub struct Search {
    pub edit: TextEdit,
    latest: Latest<Answer>,
    /// The last answer, and the query it answers.
    shown: Option<(String, Load<Vec<CatalogRow>>)>,
    /// Counts answers, so each set of result strips starts scrolled home.
    version: u64,
}

impl Default for Search {
    fn default() -> Self {
        Self {
            edit: TextEdit::new(""),
            latest: Latest::default().debounce(Duration::from_millis(300)),
            shown: None,
            version: 0,
        }
    }
}

impl Search {
    pub fn set_waker(&self, waker: Waker) {
        self.latest.set_waker(waker);
    }

    pub fn query(&self) -> &str {
        self.edit.text.trim()
    }

    /// Puts the keyboard in the box, with what's there selected.
    pub fn focus(&mut self) {
        self.edit.focused = true;
        self.edit.set_selection(0, self.edit.text.len());
    }

    /// Starts the box with a key typed while nothing had focus.
    pub fn type_first(&mut self, typed: &str) {
        self.edit.replace_text(typed);
        self.edit.set_caret(typed.len());
        self.edit.focused = true;
    }

    /// Searches for what's in the box, after the debounce.
    pub fn run_now(&mut self, core: &Core) {
        let query = self.query().to_string();
        let work = core.search(&query);
        self.latest.request((query, work), |(query, work)| {
            let rows = crate::runtime::get()
                .block_on(work)
                .map_err(|err| format!("{err:#}"));
            (query, rows)
        });
    }

    /// Waits, up to `timeout`, for a search in flight. Headless renders
    /// only, as [`super::Bg::settle`].
    pub fn settle(&self, timeout: Duration) {
        let deadline = std::time::Instant::now() + timeout;
        while self.latest.is_pending() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Takes an answer that arrived; `true` when there was one.
    pub fn land(&mut self) -> bool {
        let Some((query, rows)) = self.latest.take() else {
            return false;
        };
        self.shown = Some((query, Load::from_result(rows.map_err(anyhow::Error::msg))));
        self.version += 1;
        true
    }

    /// Shows `rows` as the answer to `query`, for scenes.
    pub fn show(&mut self, query: &str, rows: anyhow::Result<Vec<CatalogRow>>) {
        self.edit.replace_text(query);
        self.shown = Some((query.to_string(), Load::from_result(rows)));
        self.version += 1;
    }
}

impl App {
    /// Typing in the bar opens results; emptying it returns to where you
    /// were.
    pub fn search_changed(&mut self, tx: &mut Tx) {
        if self.search.query().is_empty() {
            self.search.latest.cancel();
            self.search.shown = None;
            if self.nav.page == Page::Search {
                self.show(self.nav.before_search, tx);
                self.nav.details.clear();
            }
            return;
        }
        if self.nav.page != Page::Search {
            self.nav.before_search = self.nav.page;
            self.show(Page::Search, tx);
        }
        self.nav.details.clear();
        self.nav.parked.clear();
        self.search.run_now(&self.core);
    }
}

pub fn page(s: &App, cx: &mut Cx<App>) -> El {
    let query = s.search.query();
    let shown = s
        .search
        .shown
        .as_ref()
        .filter(|(q, _)| q == query)
        .map(|(_, rows)| rows);
    let title = match shown {
        Some(_) => format!("Results for “{query}”"),
        None => "Search".to_string(),
    };
    let mut content = col()
        .key("search")
        .w_full()
        .pt(BAR_H + 28.0)
        .pb(56.0)
        .gap(space::SECTION)
        .child(page_title(title).px(space::PAGE));
    let pad = |e: El| col().w_full().px(space::PAGE).child(e);
    content = match shown {
        _ if query.is_empty() => content.child(pad(muted(
            "Type to search Cinemeta and any catalogs your sources offer.",
        ))),
        None | Some(Load::Loading) => content.child(skeleton_strip()).child(skeleton_strip()),
        Some(Load::Failed(err)) => content.child(pad(error_text(format!("Search failed: {err}")))),
        Some(Load::Ready(rows)) if rows.is_empty() => content.child(empty_state(
            &icons::SEARCH_X,
            &format!("No results for “{query}”"),
            "Try another spelling, or a shorter title.",
        )),
        Some(Load::Ready(rows)) => {
            let scale = cx.env().scale;
            rows.iter().enumerate().fold(content, |content, (i, row)| {
                content.child(home::shelf(
                    &row.title,
                    home::meta_strip(
                        s,
                        &format!("search-{}-{i}", s.search.version),
                        &row.items,
                        scale,
                    ),
                ))
            })
        }
    };
    scroller(s, Page::Search).child(content)
}
