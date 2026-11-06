//! The Rune interface: a top bar over one page at a time (Home, Library,
//! Downloads, Search, Settings), a title's details in a sheet over the page,
//! and toasts.
//!
//! One [`App`] holds everything. Views read it and build elements;
//! handlers change it, and the view runs again. Pages ask for navigation and
//! playback with an [`Action`], which is plain data, so a card or button
//! carries what it does instead of a closure.

mod bg;
mod detail;
mod dev;
mod downloads;
mod format;
mod home;
mod icons;
mod images;
mod library;
mod play;
mod poster;
mod ratings;
mod scroll;
mod search;
mod settings;
mod sources;
mod strip;
#[cfg(test)]
mod tests;
mod theme;
mod toast;
mod topbar;
mod widgets;

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use rune::{AppSpec, Cx, El, Key, KeyEvent, Live, PointerButton, Tx, Waker, el};

pub use bg::Bg;
use detail::Detail;
use images::Images;
use strip::Strips;
use toast::Toasts;

use crate::addons::Meta;
use crate::library::LibraryTitle;
use crate::state::Core;
use crate::watch::{PlayItem, WatchEntry};

pub fn run() {
    let spec = AppSpec {
        min_size: Some((960.0, 600.0)),
        scenes: dev::SCENES,
        setup: Some(|s: &mut App, waker: Waker| s.search.set_waker(waker)),
        poll: Some(poll),
        on_key: Some(on_key),
        ..AppSpec::new("Rlex", (1280.0, 800.0), || App::new(Bg::detached()), view)
    };
    // Scroll bars show whenever a page overflows, as a desktop app's do with a
    // mouse, rather than only while scrolling; `--scroll-bars` still chooses.
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if !args.iter().any(|a| a == "--scroll-bars") {
        args.extend(["--scroll-bars".into(), "always".into()]);
    }
    let app = rune::App::new(spec).args(args);
    let handle = app.handle();
    let ready = handle.clone();
    app.init(move || {
        let mut app = App::new(Bg::new(handle));
        // A headless render starts loading straight away and waits for it in
        // `poll`; a window waits for its first frame.
        if rune::is_headless() {
            crate::store::set_read_only();
            app.start(false);
        }
        app
    })
    .on_ready(move |_| ready.update(|s, _| s.start(true)))
    .run();
}

/// The pages the top bar switches between.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Library,
    Downloads,
    Search,
    Settings,
}

impl Page {
    /// The pages named in the bar; Search and Settings have their own
    /// controls.
    pub const NAV: [Page; 3] = [Page::Home, Page::Library, Page::Downloads];

    pub fn label(self) -> &'static str {
        match self {
            Page::Home => "Home",
            Page::Library => "My Library",
            Page::Downloads => "Downloads",
            Page::Search => "Search",
            Page::Settings => "Settings",
        }
    }

    /// The key of the page's scroll viewport.
    pub fn key(self) -> &'static str {
        match self {
            Page::Home => "page-home",
            Page::Library => "page-library",
            Page::Downloads => "page-downloads",
            Page::Search => "page-search",
            Page::Settings => "page-settings",
        }
    }
}

pub struct Nav {
    pub page: Page,
    /// Where emptying the search box returns to.
    pub before_search: Page,
    /// Title details over the page, the last one showing.
    pub details: Vec<Detail>,
    /// Details set aside while Settings shows on their behalf ("Add a source
    /// to play"); Back brings them back.
    pub parked: Vec<Detail>,
    /// The page the parked details were over.
    pub parked_page: Page,
}

/// What a card or button does when pressed.
#[derive(Clone, Debug)]
pub enum Action {
    Go(Page),
    Back,
    Open(DetailTarget),
    /// A movie, or a series' next-up episode, with the best source there is.
    PlayTitle(DetailTarget),
    PlayVideo(PlayItem),
    /// There's nothing to play from: Settings, to add a source, with the
    /// details set aside to come back to.
    AddSource,
}

/// What a details sheet is about: enough to draw its header before the
/// metadata arrives, and to know whether there is metadata to fetch at all.
#[derive(Clone, Debug, PartialEq)]
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

/// A slow value a page is waiting on.
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn from_result(result: anyhow::Result<T>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(err) => Self::Failed(format!("{err:#}")),
        }
    }
}

pub struct App {
    pub core: Core,
    pub bg: Bg,
    pub images: Images,
    pub strips: Strips,
    pub nav: Nav,
    pub home: home::Home,
    pub search: search::Search,
    pub settings: settings::Form,
    pub downloads: downloads::Downloads,
    pub toasts: Toasts,
    /// The title, video, file or stream whose player is starting.
    pub starting: Option<String>,
    /// How solid the top bar is: clear over the top of a page, solid once it
    /// scrolls. Pages set it as they scroll, without running the view.
    pub bar: Live<f32>,
    /// Each page's scrolling, by `Page as usize`, to come back to where
    /// it was left.
    scrolled: Rc<[Cell<f32>; 5]>,
    next_id: u64,
    started: bool,
}

impl App {
    fn new(bg: Bg) -> Self {
        Self::with(Core::load(), bg)
    }

    fn with(core: Core, bg: Bg) -> Self {
        Self {
            core,
            images: Images::new(bg.clone()),
            strips: Strips::new(bg.clone()),
            bg,
            nav: Nav {
                page: Page::Home,
                before_search: Page::Home,
                details: Vec::new(),
                parked: Vec::new(),
                parked_page: Page::Home,
            },
            home: home::Home::default(),
            search: search::Search::default(),
            settings: settings::Form::default(),
            downloads: downloads::Downloads::default(),
            toasts: Toasts::default(),
            starting: None,
            bar: Live::new(0.0),
            scrolled: Rc::default(),
            next_id: 0,
            started: false,
        }
    }

    /// The slow work of launch: home rows, then (in a window) the library
    /// rescan and torrents to resume.
    fn start(&mut self, in_window: bool) {
        if std::mem::replace(&mut self.started, true) {
            return;
        }
        log::debug!(
            "starting: home rows{}",
            if in_window {
                ", library and torrents"
            } else {
                ""
            }
        );
        self.settings.omdb_key =
            rune::TextEdit::new(self.core.settings.omdb_api_key.clone().unwrap_or_default());
        self.sync_home();
        if in_window {
            // Torrents left from last time resume downloading.
            if crate::torrent::has_saved_torrents() {
                drop(self.start_engine());
            }
            if !self.core.settings.library_folders.is_empty() {
                self.rescan(false);
            }
        }
    }

    /// A number no other sheet or toast has had, for keys and for routing
    /// late results.
    pub fn next_id(&mut self) -> u64 {
        self.next_id += 1;
        self.next_id
    }

    pub fn act(&mut self, action: Action, tx: &mut Tx) {
        match action {
            Action::Go(page) => self.go(page, tx),
            Action::Back => self.back(tx),
            Action::Open(target) => self.open_detail(target),
            Action::PlayTitle(target) => self.play_title(target),
            Action::PlayVideo(item) => self.play_video(item, None),
            Action::AddSource => {
                let details = std::mem::take(&mut self.nav.details);
                let page = self.nav.page;
                self.go(Page::Settings, tx);
                self.nav.parked = details;
                self.nav.parked_page = page;
            }
        }
    }

    fn go(&mut self, page: Page, tx: &mut Tx) {
        // Choosing the page you're on scrolls it back to the top.
        if self.nav.page == page && self.nav.details.is_empty() {
            tx.scroll_to_with(page.key(), 0.0, 0.0, theme::motion::GLIDE);
        }
        if page != Page::Search && self.nav.page == Page::Search {
            self.nav.before_search = page;
        }
        self.show(page, tx);
        self.nav.details.clear();
        self.nav.parked.clear();
    }

    /// Shows `page` where it was left scrolled.
    pub fn show(&mut self, page: Page, tx: &mut Tx) {
        if self.nav.page != page {
            self.nav.page = page;
            tx.scroll_to_instant(page.key(), 0.0, self.scrolled[page as usize].get());
        }
    }

    pub fn can_go_back(&self) -> bool {
        !self.nav.details.is_empty() || !self.nav.parked.is_empty()
    }

    fn back(&mut self, tx: &mut Tx) {
        if self.nav.details.pop().is_none() && !self.nav.parked.is_empty() {
            self.nav.details = std::mem::take(&mut self.nav.parked);
            self.show(self.nav.parked_page, tx);
        }
    }
}

/// A handler that does `action`.
pub fn on(action: Action) -> impl Fn(&mut App, &mut Tx) + 'static {
    move |s: &mut App, tx: &mut Tx| s.act(action.clone(), tx)
}

/// A page's scroll viewport over `content`. Scrolling it makes the top bar
/// solid, and where it's left is where it comes back to.
pub fn scroller(s: &App, page: Page, content: El) -> El {
    let (bar, scrolled) = (s.bar.clone(), s.scrolled.clone());
    scroll::scroll_area(page.key(), content, move |at| {
        bar.set((at.offset[1] / 96.0).clamp(0.0, 1.0));
        scrolled[page as usize].set(at.offset[1]);
    })
}

fn view(s: &App, cx: &mut Cx<App>) -> El {
    let page = match s.nav.page {
        Page::Home => home::page(s, cx),
        Page::Library => library::page(s, cx),
        Page::Downloads => downloads::page(s, cx),
        Page::Search => search::page(s, cx),
        Page::Settings => settings::page(s, cx),
    };
    let mut root = el()
        .key("app")
        .w_full()
        .h_full()
        .fill(theme::t().bg)
        .role("application")
        // The mouse's back button goes back anywhere in the window.
        .pointer_buttons([PointerButton::Back])
        .on_press_tx(|s: &mut App, tx| {
            if tx.button() == PointerButton::Back {
                s.act(Action::Back, tx);
            }
        })
        .child(page)
        .child(topbar::bar(s, cx));
    if let Some(detail) = s.nav.details.last() {
        root = root.child(detail::sheet(s, detail, cx));
    }
    if let Some(dialog) = downloads::confirm_dialog(s) {
        root = root.child(dialog);
    }
    root.child(toast::stack(s))
}

fn poll(s: &mut App, clock: f64, tx: &mut Tx) -> bool {
    if rune::is_headless() {
        s.bg.settle(Duration::from_secs(8));
        s.search.settle(Duration::from_secs(8));
    }
    let mut changed = s.strips.take_dirty();
    changed |= s.search.land();
    changed |= s.toasts.tick(clock, tx);
    changed |= s.refresh_downloads(clock, tx);
    s.images.trim();
    changed
}

fn on_key(s: &mut App, ev: &KeyEvent, tx: &mut Tx) -> bool {
    let plain = !(ev.mods.cmd || ev.mods.ctrl || ev.mods.alt);
    match &ev.key {
        Key::Escape if s.can_go_back() => {
            s.act(Action::Back, tx);
            true
        }
        Key::Command('f' | 'k') => {
            s.search.focus();
            true
        }
        Key::Command('1') => go(s, Page::Home, tx),
        Key::Command('2') => go(s, Page::Library, tx),
        Key::Command('3') => go(s, Page::Downloads, tx),
        Key::Command(',') => go(s, Page::Settings, tx),
        // A focused field takes what's typed before this sees it, so a key
        // here was typed over the page (or a button): it starts a search.
        Key::Text(typed) if plain && typed == "/" => {
            s.search.focus();
            true
        }
        Key::Text(typed) if plain && typed.chars().count() == 1 && !typed.trim().is_empty() => {
            s.search.type_first(typed);
            s.search_changed(tx);
            true
        }
        _ => false,
    }
}

fn go(s: &mut App, page: Page, tx: &mut Tx) -> bool {
    s.act(Action::Go(page), tx);
    true
}
