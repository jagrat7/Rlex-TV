//! The window's frame: a top bar laid over the page, the active page, and
//! the stack of Detail pages pushed over it. Also turns backend events into
//! notifications.
//!
//! The shell owns page scrolling, so the bar can go solid once the page
//! moves under it and each page keeps its scroll position. Search lives in
//! the bar: typing while nothing else has focus starts a search, and
//! clearing the box goes back.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::{ActiveTheme, Icon, Selectable, Sizable, StyledExt, WindowExt, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::common::TOP_BAR_HEIGHT;
use super::detail::DetailView;
use super::downloads::DownloadsView;
use super::home::HomeView;
use super::library::LibraryView;
use super::search::SearchView;
use super::settings::SettingsView;
use super::{DetailTarget, NavEvent};
use crate::state::{AppEvent, AppState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Home,
    Library,
    Downloads,
    Search,
    Settings,
}

impl Section {
    /// The sections named in the bar; Search and Settings have their own
    /// controls on the right.
    const NAV: [Section; 3] = [Section::Home, Section::Library, Section::Downloads];

    fn label(self) -> &'static str {
        match self {
            Section::Home => "Home",
            Section::Library => "My Library",
            Section::Downloads => "Downloads",
            Section::Search => "Search",
            Section::Settings => "Settings",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Past this far down, the bar stops being see-through.
const SOLID_AFTER: f32 = 24.;

pub struct Shell {
    state: Entity<AppState>,
    section: Section,
    /// Where clearing the search box returns to.
    before_search: Section,
    home: Entity<HomeView>,
    search: Entity<SearchView>,
    library: Entity<LibraryView>,
    downloads: Entity<DownloadsView>,
    settings: Entity<SettingsView>,
    /// One scroll position per section, by `Section::index`.
    scroll: [ScrollHandle; 5],
    /// Detail pages, the last one showing, each with its scroll position.
    /// Empty means the section shows.
    stack: Vec<(Entity<DetailView>, ScrollHandle)>,
    /// Detail pages set aside while Settings shows on their behalf ("Add a
    /// source to play"); Back brings them back.
    parked: Vec<(Entity<DetailView>, ScrollHandle)>,
    /// The section the parked pages were shown over.
    parked_section: Section,
    focus: FocusHandle,
    /// Take focus back on the next render, after the focused element (a
    /// button on a page that's gone) may have unmounted. Keeps Esc and
    /// type-to-search working.
    refocus: bool,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = cx.new(|cx| HomeView::new(state.clone(), cx));
        let search = cx.new(|cx| SearchView::new(state.clone(), window, cx));
        let library = cx.new(|cx| LibraryView::new(state.clone(), cx));
        let downloads = cx.new(|cx| DownloadsView::new(state.clone(), cx));
        let settings = cx.new(|cx| SettingsView::new(state.clone(), window, cx));
        let search_input = search.read(cx).input();

        let subscriptions = vec![
            cx.subscribe(&home, |this, _, event: &NavEvent, cx| {
                this.navigate(event, cx)
            }),
            cx.subscribe(&search, |this, _, event: &NavEvent, cx| {
                this.navigate(event, cx)
            }),
            cx.subscribe(&library, |this, _, event: &NavEvent, cx| {
                this.navigate(event, cx)
            }),
            cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let empty = input.read(cx).value().trim().is_empty();
                    this.search_changed(empty, cx);
                }
            }),
            cx.subscribe_in(&state, window, |_, _, event: &AppEvent, window, cx| {
                let note = match event {
                    AppEvent::Notice(message) => Notification::success(message.clone()),
                    AppEvent::Error(message) => Notification::error(message.clone()),
                };
                window.push_notification(note, cx);
            }),
        ];

        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        // When the focused element leaves the window (a button that hid
        // itself), take focus back so Esc and type-to-search keep working.
        cx.on_focus_lost(window, |this, window, cx| window.focus(&this.focus, cx))
            .detach();
        let mut this = Self {
            state,
            section: Section::Home,
            before_search: Section::Home,
            home,
            search,
            library,
            downloads,
            settings,
            scroll: Default::default(),
            stack: Vec::new(),
            parked: Vec::new(),
            parked_section: Section::Home,
            focus,
            refocus: false,
            _subscriptions: subscriptions,
        };
        #[cfg(debug_assertions)]
        this.apply_dev_route(window, cx);
        this
    }

    /// Debug builds open the page named by `RLEX_DEV_VIEW`, so each page can
    /// be screenshotted without clicking through: `search:<query>`,
    /// `library`, `downloads`, `settings`, or
    /// `detail:<kind>:<id>[@<episode video id>]` (ids contain colons).
    #[cfg(debug_assertions)]
    fn apply_dev_route(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Ok(route) = std::env::var("RLEX_DEV_VIEW") else {
            return;
        };
        let (page, arg) = route.split_once(':').unwrap_or((route.as_str(), ""));
        match (page, arg.split_once(':')) {
            ("search", _) => {
                self.section = Section::Search;
                let query = arg.to_string();
                self.search
                    .update(cx, |search, cx| search.set_query(&query, window, cx));
            }
            ("library", _) => self.section = Section::Library,
            ("downloads", _) => self.section = Section::Downloads,
            ("settings", _) => self.section = Section::Settings,
            ("detail", Some((kind, rest))) => {
                let (id, episode) = match rest.split_once('@') {
                    Some((id, episode)) => (id, Some(episode)),
                    None => (rest, None),
                };
                let target = DetailTarget {
                    kind: kind.to_string(),
                    id: id.to_string(),
                    name: id.to_string(),
                    poster: None,
                };
                self.push_detail(target, cx);
                if let (Some((view, _)), Some(episode)) = (self.stack.last(), episode) {
                    let episode = episode.to_string();
                    view.update(cx, |view, _| view.preselect_episode(episode));
                }
            }
            _ => log::warn!("RLEX_DEV_VIEW: unknown route {route:?}"),
        }
    }

    fn navigate(&mut self, event: &NavEvent, cx: &mut Context<Self>) {
        match event {
            NavEvent::OpenDetail(target) => self.push_detail(target.clone(), cx),
            NavEvent::OpenSettings => {
                let stack = std::mem::take(&mut self.stack);
                let section = self.section;
                self.show(Section::Settings, cx);
                self.parked = stack;
                self.parked_section = section;
            }
        }
    }

    fn push_detail(&mut self, target: DetailTarget, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let view = cx.new(|cx| DetailView::new(state, target, cx));
        cx.subscribe(&view, |this, _, event: &NavEvent, cx| {
            this.navigate(event, cx)
        })
        .detach();
        self.stack.push((view, ScrollHandle::new()));
        self.refocus = true;
        cx.notify();
    }

    fn can_go_back(&self) -> bool {
        !self.stack.is_empty() || !self.parked.is_empty()
    }

    fn pop(&mut self, cx: &mut Context<Self>) {
        if self.stack.pop().is_none() && !self.parked.is_empty() {
            self.stack = std::mem::take(&mut self.parked);
            self.section = self.parked_section;
        }
        self.refocus = true;
        cx.notify();
    }

    fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        // Choosing the page you're on scrolls it back to the top.
        if self.stack.is_empty() && self.section == section {
            self.scroll[section.index()].set_offset(point(px(0.), px(0.)));
        }
        if section != Section::Search && self.section == Section::Search {
            self.before_search = section;
        }
        self.section = section;
        self.stack.clear();
        self.parked.clear();
        self.refocus = true;
        cx.notify();
    }

    /// Typing in the bar opens results; emptying it returns to where you
    /// were.
    fn search_changed(&mut self, empty: bool, cx: &mut Context<Self>) {
        if !empty {
            if self.section != Section::Search {
                self.before_search = self.section;
                self.section = Section::Search;
            }
            self.stack.clear();
            self.parked.clear();
        } else if self.section == Section::Search {
            self.section = self.before_search;
            self.stack.clear();
        }
        cx.notify();
    }

    fn current_scroll(&self) -> &ScrollHandle {
        match self.stack.last() {
            Some((_, handle)) => handle,
            None => &self.scroll[self.section.index()],
        }
    }

    fn top_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let on_page = self.stack.is_empty();
        let can_go_back = self.can_go_back();
        let solid = self.current_scroll().offset().y < px(-SOLID_AFTER);
        let bg = theme.background;
        let search_input = self.search.read(cx).input();

        let nav = h_flex().gap_5().children(Section::NAV.map(|section| {
            let active = on_page && self.section == section;
            div()
                .id(section.label())
                .text_sm()
                .cursor_pointer()
                .text_color(if active {
                    theme.foreground
                } else {
                    theme.foreground.alpha(0.7)
                })
                .when(active, |this| this.font_semibold())
                .hover(|this| this.text_color(theme.foreground))
                .child(section.label())
                .on_click(cx.listener(move |this, _, _, cx| this.show(section, cx)))
        }));

        h_flex()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .h(px(TOP_BAR_HEIGHT))
            .px_6()
            .gap_6()
            .items_center()
            .map(|this| {
                if solid {
                    this.bg(bg.alpha(0.97))
                        .border_b_1()
                        .border_color(theme.border.alpha(0.6))
                } else {
                    this.bg(linear_gradient(
                        180.,
                        linear_color_stop(bg.alpha(0.85), 0.),
                        linear_color_stop(bg.alpha(0.0), 1.),
                    ))
                }
            })
            .when(can_go_back, |this| {
                this.child(
                    Button::new("back")
                        .ghost()
                        .small()
                        .icon(IconName::ArrowLeft)
                        .tooltip("Back (Esc)")
                        .on_click(cx.listener(|this, _, _, cx| this.pop(cx))),
                )
            })
            .child(
                h_flex()
                    .id("brand")
                    .gap_2()
                    .items_center()
                    .cursor_pointer()
                    .child(
                        img(super::logo())
                            .size_7()
                            .flex_shrink_0()
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(div().text_lg().font_bold().child("Rlex"))
                    .on_click(cx.listener(|this, _, _, cx| this.show(Section::Home, cx))),
            )
            .child(nav)
            .child(div().flex_1())
            .child(
                div().w(px(280.)).child(
                    Input::new(&search_input)
                        .small()
                        .prefix(Icon::new(IconName::Search).text_color(theme.muted_foreground))
                        .cleanable(true),
                ),
            )
            .child(
                Button::new("settings")
                    .ghost()
                    .small()
                    .icon(IconName::Settings)
                    .tooltip("Settings")
                    .selected(on_page && self.section == Section::Settings)
                    .on_click(cx.listener(|this, _, _, cx| this.show(Section::Settings, cx))),
            )
    }

    fn content(&self) -> AnyElement {
        if let Some((detail, _)) = self.stack.last() {
            return detail.clone().into_any_element();
        }
        match self.section {
            Section::Home => self.home.clone().into_any_element(),
            Section::Search => self.search.clone().into_any_element(),
            Section::Library => self.library.clone().into_any_element(),
            Section::Downloads => self.downloads.clone().into_any_element(),
            Section::Settings => self.settings.clone().into_any_element(),
        }
    }

    /// Each page and detail scrolls on its own, keyed so the positions stay
    /// apart.
    fn scroll_id(&self) -> ElementId {
        match self.stack.len() {
            0 => ElementId::Name(format!("page-{}", self.section.label()).into()),
            depth => ElementId::Name(format!("detail-{depth}").into()),
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if std::mem::take(&mut self.refocus) {
            window.focus(&self.focus, cx);
        }
        let scroll = self.current_scroll().clone();
        div()
            .id("shell")
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                // Keys in text fields (search, Settings) are the field's own;
                // Esc in the search box clears it.
                let in_text_field = window
                    .context_stack()
                    .iter()
                    .any(|context| context.contains("Input"));
                if event.keystroke.key == "escape" && !in_text_field && this.can_go_back() {
                    this.pop(cx);
                    cx.stop_propagation();
                    return;
                }
                // Typing with nothing else focused starts a search.
                let modifiers = &event.keystroke.modifiers;
                let plain = !(modifiers.control || modifiers.alt || modifiers.platform);
                let typed = event
                    .keystroke
                    .key_char
                    .as_deref()
                    .filter(|c| plain && c.chars().count() == 1 && !c.trim().is_empty());
                if let Some(text) = typed
                    && !in_text_field
                {
                    let text = text.to_string();
                    let input = this.search.read(cx).input();
                    // `replace_all` emits Change (`set_value` doesn't), which
                    // opens the results and starts the search.
                    let handle = input.update(cx, |input, cx| {
                        input.replace_all(text, window, cx);
                        input.focus_handle(cx)
                    });
                    window.focus(&handle, cx);
                    cx.stop_propagation();
                }
            }))
            .on_mouse_down(
                MouseButton::Navigate(NavigationDirection::Back),
                cx.listener(|this, _, _, cx| this.pop(cx)),
            )
            .child(
                div()
                    .id(self.scroll_id())
                    .size_full()
                    .overflow_y_scroll()
                    .track_scroll(&scroll)
                    .child(self.content())
                    .vertical_scrollbar(&scroll),
            )
            .child(self.top_bar(cx))
    }
}
