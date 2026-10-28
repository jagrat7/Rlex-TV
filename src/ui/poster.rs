//! The poster card: a 2:3 image with the title and year under it, used in
//! horizontal rows and wrapping grids everywhere a title is listed.

use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::{ActiveTheme, StyledExt, h_flex, v_flex};
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
    on_click: Option<ClickHandler>,
}

impl PosterCard {
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            subtitle: None,
            poster: None,
            progress: None,
            on_click: None,
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

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }
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

        v_flex()
            .id(self.id)
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
                .child(title),
        )
        .into_any_element()
}

/// A horizontally scrolling row of cards.
pub fn poster_row(
    id: &'static str,
    cards: impl IntoIterator<Item = PosterCard>,
) -> impl IntoElement {
    div().w_full().h(px(ROW_HEIGHT)).child(
        h_flex()
            .gap_4()
            .items_start()
            .pb_3()
            .overflow_x_scrollbar()
            .id(id)
            .children(cards),
    )
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
