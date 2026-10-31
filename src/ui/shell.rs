//! The window's frame: the sidebar, the active page, and the stack of Detail
//! pages pushed over it. Also turns backend events into notifications.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::sidebar::{
    Sidebar, SidebarCollapsible, SidebarHeader, SidebarMenu, SidebarMenuItem,
};
use gpui_kit::component::{ActiveTheme, StyledExt, WindowExt, h_flex, v_flex};
use gpui_kit::*;

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
    Search,
    Library,
    Downloads,
    Settings,
}

impl Section {
    const ALL: [Section; 5] = [
        Section::Home,
        Section::Search,
        Section::Library,
        Section::Downloads,
        Section::Settings,
    ];

    fn label(self) -> &'static str {
        match self {
            Section::Home => "Home",
            Section::Search => "Search",
            Section::Library => "Library",
            Section::Downloads => "Downloads",
            Section::Settings => "Settings",
        }
    }

    fn icon(self) -> IconName {
        match self {
            Section::Home => IconName::House,
            Section::Search => IconName::Search,
            Section::Library => IconName::Library,
            Section::Downloads => IconName::Download,
            Section::Settings => IconName::Settings,
        }
    }
}

pub struct Shell {
    state: Entity<AppState>,
    section: Section,
    home: Entity<HomeView>,
    search: Entity<SearchView>,
    library: Entity<LibraryView>,
    downloads: Entity<DownloadsView>,
    settings: Entity<SettingsView>,
    /// Detail pages, the last one showing. Empty means the section shows.
    stack: Vec<Entity<DetailView>>,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let home = cx.new(|cx| HomeView::new(state.clone(), cx));
        let search = cx.new(|cx| SearchView::new(state.clone(), window, cx));
        let library = cx.new(|cx| LibraryView::new(state.clone(), cx));
        let downloads = cx.new(|cx| DownloadsView::new(state.clone(), cx));
        let settings = cx.new(|cx| SettingsView::new(state.clone(), window, cx));

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
            cx.subscribe_in(&state, window, |_, _, event: &AppEvent, window, cx| {
                let note = match event {
                    AppEvent::Notice(message) => Notification::success(message.clone()),
                    AppEvent::Error(message) => Notification::error(message.clone()),
                };
                window.push_notification(note, cx);
            }),
        ];

        let mut this = Self {
            state,
            section: Section::Home,
            home,
            search,
            library,
            downloads,
            settings,
            stack: Vec::new(),
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
                if let (Some(view), Some(episode)) = (self.stack.last(), episode) {
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
        }
    }

    fn push_detail(&mut self, target: DetailTarget, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let view = cx.new(|cx| DetailView::new(state, target, cx));
        self.stack.push(view);
        cx.notify();
    }

    fn pop(&mut self, cx: &mut Context<Self>) {
        self.stack.pop();
        cx.notify();
    }

    fn show(&mut self, section: Section, cx: &mut Context<Self>) {
        self.section = section;
        self.stack.clear();
        cx.notify();
    }

    fn sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let on_page = self.stack.is_empty();
        Sidebar::new("sidebar")
            .collapsible(SidebarCollapsible::None)
            .w(px(216.))
            .header(
                SidebarHeader::new()
                    .child(
                        img(super::logo())
                            .size_8()
                            .flex_shrink_0()
                            .object_fit(ObjectFit::Contain),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .overflow_hidden()
                            .child(div().font_semibold().child("Rlex"))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child("Media center"),
                            ),
                    ),
            )
            .child(SidebarMenu::new().children(Section::ALL.map(|section| {
                SidebarMenuItem::new(section.label())
                    .icon(section.icon())
                    .active(on_page && self.section == section)
                    .on_click(cx.listener(move |this, _, _, cx| this.show(section, cx)))
            })))
    }

    fn content(&self, cx: &mut Context<Self>) -> AnyElement {
        if let Some(detail) = self.stack.last() {
            return v_flex()
                .size_full()
                .child(
                    h_flex().px_6().pt_4().child(
                        Button::new("back")
                            .ghost()
                            .icon(IconName::ArrowLeft)
                            .label("Back")
                            .on_click(cx.listener(|this, _, _, cx| this.pop(cx))),
                    ),
                )
                .child(div().flex_1().min_h_0().child(detail.clone()))
                .into_any_element();
        }
        match self.section {
            Section::Home => self.home.clone().into_any_element(),
            Section::Search => self.search.clone().into_any_element(),
            Section::Library => self.library.clone().into_any_element(),
            Section::Downloads => self.downloads.clone().into_any_element(),
            Section::Settings => self.settings.clone().into_any_element(),
        }
    }
}

impl Render for Shell {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.sidebar(cx))
            .child(div().h_full().flex_1().min_w_0().child(self.content(cx)))
    }
}
