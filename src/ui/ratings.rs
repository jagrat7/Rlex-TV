//! Rating badges: an IMDb pill, a Rotten Tomatoes tomato and a Metacritic
//! square, each left out when its score is missing.
//!
//! ```ignore
//! RatingBadges::new()
//!     .imdb(meta.imdb_rating.clone())
//!     .rotten_tomatoes(Some(93))
//!     .metacritic(Some(74))
//!     .small()
//! ```

use gpui_kit::component::{StyledExt, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use crate::ratings::Ratings;

const IMDB_YELLOW: u32 = 0xF5C518;
const TOMATO_RED: u32 = 0xFA320A;
const TOMATO_STEM: u32 = 0x00912D;
const ROTTEN_GREEN: u32 = 0x0AC855;
const META_GREEN: u32 = 0x66CC33;
const META_YELLOW: u32 = 0xFFCC33;
const META_RED: u32 = 0xFF0000;

/// The Tomatometer's "fresh" line.
const FRESH_FROM: u8 = 60;

#[derive(IntoElement, Default)]
pub struct RatingBadges {
    imdb: Option<SharedString>,
    rotten_tomatoes: Option<u8>,
    metacritic: Option<u8>,
    small: bool,
}

impl RatingBadges {
    pub fn new() -> Self {
        Self::default()
    }

    /// Out of ten, as text ("8.8"). Blank values are ignored.
    pub fn imdb(mut self, score: Option<impl Into<SharedString>>) -> Self {
        self.imdb = score
            .map(Into::into)
            .filter(|s: &SharedString| !s.trim().is_empty() && s.trim() != "N/A");
        self
    }

    /// Tomatometer percent.
    pub fn rotten_tomatoes(mut self, percent: Option<u8>) -> Self {
        self.rotten_tomatoes = percent;
        self
    }

    /// Metascore, out of 100.
    pub fn metacritic(mut self, score: Option<u8>) -> Self {
        self.metacritic = score;
        self
    }

    /// Compact sizing, for cards and dense rows.
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.imdb.is_none() && self.rotten_tomatoes.is_none() && self.metacritic.is_none()
    }

    /// Same as the free [`from_ratings`].
    pub fn from_ratings(r: Option<&Ratings>, fallback_imdb: Option<&str>) -> Self {
        from_ratings(r, fallback_imdb)
    }
}

/// Badges for OMDb scores, using Cinemeta's IMDb score when OMDb has none
/// (or there is no OMDb key).
pub fn from_ratings(r: Option<&Ratings>, fallback_imdb: Option<&str>) -> RatingBadges {
    let imdb = r
        .and_then(|r| r.imdb.clone())
        .or_else(|| fallback_imdb.map(str::to_string));
    RatingBadges::new()
        .imdb(imdb)
        .rotten_tomatoes(r.and_then(|r| r.rotten_tomatoes))
        .metacritic(r.and_then(|r| r.metacritic))
}

impl RenderOnce for RatingBadges {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        let small = self.small;
        let text = |div: Div| if small { div.text_xs() } else { div.text_sm() };

        let imdb = self.imdb.map(|score| {
            h_flex()
                .items_center()
                .gap(px(if small { 4. } else { 6. }))
                .child(
                    text(div())
                        .px(px(if small { 3. } else { 4. }))
                        .rounded(px(3.))
                        .bg(rgb(IMDB_YELLOW))
                        .text_color(rgb(0x000000))
                        .font_black()
                        .line_height(relative(1.25))
                        .child("IMDb"),
                )
                .child(text(div()).font_semibold().child(score))
        });

        let tomato = self.rotten_tomatoes.map(|percent| {
            let size = px(if small { 12. } else { 15. });
            let fresh = percent >= FRESH_FROM;
            let icon = if fresh {
                // A red tomato with a little green stem on top.
                div()
                    .relative()
                    .size(size)
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .rounded_full()
                            .bg(rgb(TOMATO_RED)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top(px(-1.))
                            .left(size * 0.3)
                            .w(size * 0.4)
                            .h(size * 0.25)
                            .rounded(px(2.))
                            .bg(rgb(TOMATO_STEM)),
                    )
            } else {
                // A green splat: a blob with a couple of drops.
                div()
                    .relative()
                    .size(size)
                    .child(
                        div()
                            .absolute()
                            .top(size * 0.15)
                            .left(size * 0.1)
                            .size(size * 0.8)
                            .rounded_full()
                            .bg(rgb(ROTTEN_GREEN)),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .size(size * 0.3)
                            .rounded_full()
                            .bg(rgb(ROTTEN_GREEN)),
                    )
                    .child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .size(size * 0.25)
                            .rounded_full()
                            .bg(rgb(ROTTEN_GREEN)),
                    )
            };
            h_flex()
                .items_center()
                .gap(px(if small { 4. } else { 5. }))
                .child(icon)
                .child(text(div()).font_semibold().child(format!("{percent}%")))
        });

        let meta = self.metacritic.map(|score| {
            let color = match score {
                61.. => META_GREEN,
                40..=60 => META_YELLOW,
                _ => META_RED,
            };
            let foreground = if score >= 40 { 0x000000 } else { 0xFFFFFF };
            text(div())
                .flex()
                .items_center()
                .justify_center()
                .min_w(px(if small { 18. } else { 22. }))
                .h(px(if small { 16. } else { 20. }))
                .px(px(2.))
                .rounded(px(2.))
                .bg(rgb(color))
                .text_color(rgb(foreground))
                .font_bold()
                .child(score.to_string())
        });

        h_flex()
            .items_center()
            .flex_wrap()
            .gap(px(if small { 8. } else { 12. }))
            .when_some(imdb, |this, el| this.child(el))
            .when_some(tomato, |this, el| this.child(el))
            .when_some(meta, |this, el| this.child(el))
    }
}
