//! The poster card: a 2:3 image with the title and year under it, used in
//! horizontal rows and wrapping grids everywhere a title is listed.
//!
//! Hovering a card shows its quick actions: a round Play button that starts
//! the title straight away, and an info button when clicking the card
//! itself already plays (Continue watching).

use gpui_kit::assets::IconName;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

pub const CARD_WIDTH: f32 = 150.;
pub const POSTER_HEIGHT: f32 = 225.;
/// The card plus its two text lines and the row's scrollbar.
const ROW_HEIGHT: f32 = POSTER_HEIGHT + 64.;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct PosterCard {
    id: ElementId,
    title: SharedString,
    subtitle: Option<SharedString>,
    poster: Option<SharedString>,
    progress: Option<f32>,
    rating: Option<SharedString>,
    busy: bool,
    on_click: Option<ClickHandler>,
    on_play: Option<ClickHandler>,
    on_info: Option<ClickHandler>,
}

impl PosterCard {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            poster: None,
            progress: None,
            rating: None,
            busy: false,
            on_click: None,
            on_play: None,
            on_info: None,
        }
    }

    /// The year, or an episode label; shown under the title.
    pub fn subtitle(mut self, subtitle: Option<impl Into<SharedString>>) -> Self {
        self.subtitle = subtitle.map(Into::into);
        self
    }

    pub fn poster(mut self, url: Option<impl Into<SharedString>>) -> Self {
        self.poster = url.map(Into::into);
        self
    }

    /// 0–1; draws a thin bar along the bottom of the poster.
    pub fn progress(mut self, progress: Option<f32>) -> Self {
        self.progress = progress.filter(|p| *p > 0.0);
        self
    }

    /// The IMDb score, shown in the poster's top-left corner.
    pub fn rating(mut self, rating: Option<impl Into<SharedString>>) -> Self {
        self.rating = rating.map(Into::into);
        self
    }

    /// Covers the poster with a spinner while its player starts.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    /// A round Play button on hover, for cards whose click opens details.
    pub fn on_play(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_play = Some(Box::new(handler));
        self
    }

    /// An info button on hover, for cards whose click plays.
    pub fn on_info(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_info = Some(Box::new(handler));
        self
    }
}

/// A round button floating on the poster; its click doesn't reach the card.
fn overlay_button(
    id: impl Into<ElementId>,
    icon: IconName,
    filled: bool,
    handler: ClickHandler,
) -> Stateful<Div> {
    let (bg, fg, hover_bg) = if filled {
        (white(), black(), white().alpha(0.85))
    } else {
        (black().alpha(0.6), white(), black().alpha(0.8))
    };
    div()
        .id(id.into())
        .size(px(36.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(bg)
        .border_1()
        .border_color(white().alpha(0.7))
        .shadow_md()
        .hover(move |this| this.bg(hover_bg))
        .child(Icon::new(icon).size_4().text_color(fg))
        .on_click(move |event, window, cx| {
            cx.stop_propagation();
            handler(event, window, cx);
        })
}

impl RenderOnce for PosterCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let radius = theme.radius_lg;
        let fallback_bg = theme.muted;
        let fallback_fg = theme.muted_foreground;
        let title = self.title.clone();
        let fallback = move || fallback_tile(title.clone(), fallback_bg, fallback_fg);

        let image: AnyElement = match self.poster.clone() {
            Some(url) => {
                let loading_bg = theme.skeleton;
                img(url)
                    .size_full()
                    .object_fit(ObjectFit::Cover)
                    .with_fallback(fallback)
                    .with_loading(move || div().size_full().bg(loading_bg).into_any_element())
                    .into_any_element()
            }
            None => fallback(),
        };

        let group: SharedString = format!("poster-{:?}", self.id).into();
        let plays_on_click = self.on_info.is_some();
        let play_button = self
            .on_play
            .map(|handler| overlay_button("play", IconName::Play, true, handler));
        let info_button = self
            .on_info
            .map(|handler| overlay_button("info", IconName::Info, false, handler));
        let hover_group = group.clone();
        let overlay = div()
            .absolute()
            .inset_0()
            .invisible()
            .group_hover(hover_group, |this| this.visible())
            .bg(linear_gradient(
                180.,
                linear_color_stop(black().alpha(0.0), 0.45),
                linear_color_stop(black().alpha(0.75), 1.),
            ))
            .when(plays_on_click, |this| {
                this.child(
                    div()
                        .absolute()
                        .inset_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .size(px(52.))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_full()
                                .bg(black().alpha(0.55))
                                .border_2()
                                .border_color(white())
                                .child(Icon::new(IconName::Play).size_6().text_color(white())),
                        ),
                )
            })
            .child(
                h_flex()
                    .absolute()
                    .left_2()
                    .right_2()
                    .bottom_3()
                    .justify_between()
                    .children(play_button)
                    .child(div())
                    .children(info_button),
            );

        v_flex()
            .id(self.id)
            .group(group)
            .w(px(CARD_WIDTH))
            .flex_none()
            .gap_1p5()
            .cursor_pointer()
            .when_some(self.on_click, |this, handler| this.on_click(handler))
            .child(
                div()
                    .relative()
                    .w_full()
                    .h(px(POSTER_HEIGHT))
                    .rounded(radius)
                    .overflow_hidden()
                    .bg(theme.muted)
                    .border_2()
                    .border_color(theme.transparent)
                    .hover(|this| this.border_color(theme.primary))
                    .child(image)
                    // A fixed corner, so cards without a score line up.
                    .when_some(self.rating, |this, rating| {
                        this.child(
                            div()
                                .absolute()
                                .top_1p5()
                                .left_1p5()
                                .px_1p5()
                                .py_0p5()
                                .rounded(px(4.))
                                .bg(black().alpha(0.72))
                                .text_color(white())
                                .child(
                                    super::ratings::RatingBadges::new()
                                        .imdb(Some(rating))
                                        .small(),
                                ),
                        )
                    })
                    .when(!self.busy, |this| this.child(overlay))
                    .when(self.busy, |this| {
                        this.child(
                            div()
                                .absolute()
                                .inset_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .bg(black().alpha(0.6))
                                .child(Spinner::new().large().color(white())),
                        )
                    })
                    .when_some(self.progress, |this, progress| {
                        this.child(
                            div()
                                .absolute()
                                .bottom_0()
                                .left_0()
                                .right_0()
                                .h(px(4.))
                                .bg(black().alpha(0.55))
                                .child(
                                    div()
                                        .h_full()
                                        .w(relative(progress.clamp(0.0, 1.0)))
                                        .bg(theme.primary),
                                ),
                        )
                    }),
            )
            .child(
                div()
                    .w_full()
                    .text_sm()
                    .font_medium()
                    .truncate()
                    .child(self.title),
            )
            .child(
                div()
                    .w_full()
                    .h_4()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .when_some(self.subtitle, |this, subtitle| this.child(subtitle)),
            )
    }
}

/// Stands in for a missing or broken poster: the title on a flat tile.
fn fallback_tile(title: SharedString, bg: Hsla, fg: Hsla) -> AnyElement {
    div()
        .size_full()
        .flex()
        .items_center()
        .justify_center()
        .p_3()
        .bg(bg)
        .child(
            div()
                .text_sm()
                .font_medium()
                .text_center()
                .text_color(fg)
                .line_clamp(5)
                .text_ellipsis()
                .child(title),
        )
        .into_any_element()
}

/// A horizontally scrolling row of cards, with arrows at its edges on
/// hover that page through it a screenful at a time. Each row keeps its own
/// scroll position under `id`.
pub fn poster_row(
    id: impl Into<ElementId>,
    cards: impl IntoIterator<Item = PosterCard>,
    window: &mut Window,
    cx: &mut App,
) -> impl IntoElement {
    let id: ElementId = id.into();
    let handle = window
        .use_keyed_state(id.clone(), cx, |_, _| ScrollHandle::new())
        .read(cx)
        .clone();
    let group: SharedString = format!("row-{id}").into();
    let offset = -handle.offset().x;
    let max = handle.max_offset().x;
    let can_back = offset > px(1.);
    let can_forward = offset < max - px(1.);
    let bg = cx.theme().background;

    div()
        .group(group.clone())
        .relative()
        .w_full()
        .h(px(ROW_HEIGHT))
        .child(
            h_flex()
                .id(id.clone())
                .size_full()
                .gap_4()
                .items_start()
                .pb_3()
                .overflow_x_scroll()
                .track_scroll(&handle)
                .children(cards),
        )
        .when(can_back, |this| {
            this.child(row_arrow(&id, &group, &handle, false, bg))
        })
        .when(can_forward, |this| {
            this.child(row_arrow(&id, &group, &handle, true, bg))
        })
}

/// The edge arrow that pages a row; shown while the row is hovered.
fn row_arrow(
    row: &ElementId,
    group: &SharedString,
    handle: &ScrollHandle,
    forward: bool,
    bg: Hsla,
) -> impl IntoElement {
    let handle = handle.clone();
    let (from, to) = (bg.alpha(0.0), bg.alpha(0.92));
    div()
        .id(ElementId::Name(
            format!("{row}-{}", if forward { "next" } else { "prev" }).into(),
        ))
        .absolute()
        .top_0()
        .h(px(POSTER_HEIGHT))
        .w(px(56.))
        .when(forward, |this| this.right_0())
        .when(!forward, |this| this.left_0())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .invisible()
        .group_hover(group.clone(), |this| this.visible())
        .bg(if forward {
            linear_gradient(90., linear_color_stop(from, 0.), linear_color_stop(to, 0.7))
        } else {
            linear_gradient(90., linear_color_stop(to, 0.3), linear_color_stop(from, 1.))
        })
        .child(
            div()
                .size(px(36.))
                .flex()
                .items_center()
                .justify_center()
                .rounded_full()
                .bg(black().alpha(0.6))
                .border_1()
                .border_color(white().alpha(0.5))
                .hover(|this| this.bg(black().alpha(0.85)))
                .child(
                    Icon::new(if forward {
                        IconName::ChevronRight
                    } else {
                        IconName::ChevronLeft
                    })
                    .size_5()
                    .text_color(white()),
                ),
        )
        .on_click(move |_, window, _| {
            let step = (handle.bounds().size.width - px(CARD_WIDTH)).max(px(CARD_WIDTH));
            let current = handle.offset();
            let max = handle.max_offset().x;
            let x = if forward {
                (current.x - step).max(-max)
            } else {
                (current.x + step).min(px(0.))
            };
            handle.set_offset(point(x, current.y));
            window.refresh();
        })
}

/// Cards wrapping to the available width.
pub fn poster_grid(cards: impl IntoIterator<Item = PosterCard>) -> impl IntoElement {
    div().flex().flex_wrap().gap_4().children(cards)
}

/// A row's silhouette while its items load; the same height as a real row,
/// so nothing moves when the posters arrive.
pub fn skeleton_row(cx: &App) -> impl IntoElement {
    let radius = cx.theme().radius_lg;
    div().w_full().h(px(ROW_HEIGHT)).overflow_hidden().child(
        h_flex().gap_4().items_start().children((0..8).map(|_| {
            v_flex()
                .w(px(CARD_WIDTH))
                .flex_none()
                .gap_1p5()
                .child(
                    Skeleton::new()
                        .w_full()
                        .h(px(POSTER_HEIGHT))
                        .rounded(radius),
                )
                .child(Skeleton::new().w_3_4().h_4())
                .child(Skeleton::new().w_1_2().h_3())
        })),
    )
}
