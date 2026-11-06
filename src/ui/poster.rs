//! The poster card: a 2:3 image with the title and a line under it, used
//! everywhere a title is listed.
//!
//! Hovering a card rings it and shows its quick actions: a round Play that
//! starts the title straight away, and an info button when clicking the card
//! itself already plays (Continue watching).

use rune::{Color, Cursor, El, SurfaceFit, col, el, icon, text};

use super::images::px;
use super::theme::{t, ty};
use super::widgets::{Button, Size, Variant, cover, progress, skeleton, spinner};
use super::{Action, App, icons};

pub const CARD_W: f32 = 156.0;
pub const POSTER_H: f32 = CARD_W * 1.5;
const RADIUS: f32 = 10.0;

pub struct Card {
    key: String,
    title: String,
    subtitle: Option<String>,
    poster: Option<String>,
    progress: Option<f32>,
    rating: Option<String>,
    busy: bool,
    open: Option<Action>,
    play: Option<Action>,
    info: Option<Action>,
    width: f32,
}

impl Card {
    pub fn new(key: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            title: title.into(),
            subtitle: None,
            poster: None,
            progress: None,
            rating: None,
            busy: false,
            open: None,
            play: None,
            info: None,
            width: CARD_W,
        }
    }

    /// The year, or an episode label; shown under the title.
    pub fn subtitle(mut self, subtitle: Option<String>) -> Self {
        self.subtitle = subtitle;
        self
    }
    pub fn poster(mut self, url: Option<&str>) -> Self {
        self.poster = url.map(str::to_string);
        self
    }
    /// 0–1; a thin bar along the bottom of the poster.
    pub fn progress(mut self, progress: Option<f32>) -> Self {
        self.progress = progress.filter(|p| *p > 0.0);
        self
    }
    /// The IMDb score, in the poster's top-left corner.
    pub fn rating(mut self, rating: Option<&str>) -> Self {
        self.rating = rating
            .filter(|r| !r.trim().is_empty() && *r != "N/A")
            .map(str::to_string);
        self
    }
    /// Covers the poster with a spinner while its player starts.
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }
    /// What clicking the card does.
    pub fn on_open(mut self, action: Action) -> Self {
        self.open = Some(action);
        self
    }
    /// A round Play on hover, for cards whose click opens details.
    pub fn on_play(mut self, action: Action) -> Self {
        self.play = Some(action);
        self
    }
    /// An info button on hover, for cards whose click plays.
    pub fn on_info(mut self, action: Action) -> Self {
        self.info = Some(action);
        self
    }
    /// Wider or narrower than a strip's cards, for grids that fill their row.
    pub fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    pub fn build(self, s: &App, scale: f32) -> El {
        let th = t();
        let (w, h) = (self.width, self.width * 1.5);
        let art = self
            .poster
            .as_deref()
            .map(|url| s.images.get(url, px([w, h], scale)));
        let image = match &art {
            Some(art) if !art.failed => el()
                .w_full()
                .h_full()
                .pixels_fit(&art.pixels, SurfaceFit::Fill),
            _ => stand_in(&self.title),
        };

        let mut frame = el()
            .key("poster")
            .w(w)
            .h(h)
            .radius(RADIUS)
            .fill(th.raised2)
            .clip_rounded(RADIUS)
            .child(image);
        if let Some(score) = &self.rating {
            frame = frame.child(
                el().absolute(8.0, 8.0)
                    .px(5.0)
                    .py(3.0)
                    .radius(5.0)
                    .fill(Color::BLACK.alpha(0.72))
                    .child(super::ratings::badges(None, Some(score), true).unwrap_or_else(el)),
            );
        }
        if self.busy {
            frame = frame.child(
                cover()
                    .fill(Color::BLACK.alpha(0.6))
                    .center()
                    .child(spinner(30.0, Color::WHITE)),
            );
        } else {
            frame = frame.child(self.hover_layer());
        }
        if let Some(value) = self.progress {
            frame = frame.child(
                el().pin(Some(0.0), None, Some(0.0), Some(0.0))
                    .h(4.0)
                    .child(progress(value, 4.0, Color::BLACK.alpha(0.6), th.accent)),
            );
        }
        // On top of everything, so it frames the artwork.
        frame = frame.child(
            cover()
                .radius(RADIUS)
                .border(2.0, th.accent)
                .group_opacity(0.0, 1.0),
        );

        let mut card = col()
            .key(self.key)
            .w(w)
            .gap(8.0)
            .hover_group()
            .role("button")
            .label(self.title.clone())
            .child(frame)
            .child(
                text(self.title.clone())
                    .font(ty::SMALL_STRONG)
                    .color(th.text)
                    .truncate()
                    .help_when_truncated(self.title.clone()),
            )
            .child(
                text(self.subtitle.unwrap_or_default())
                    .font(ty::CAPTION)
                    .color(th.muted)
                    .truncate()
                    .h(16.0),
            );
        if let Some(action) = self.open {
            card = card
                .cursor(Cursor::Pointer)
                .focusable(true)
                .focus_ring(2.0, 3.0, th.accent)
                .press_scale(0.97)
                .on_press_tx(move |s: &mut App, tx| s.act(action.clone(), tx));
        }
        card
    }

    /// The scrim and quick actions that show while the card is hovered.
    fn hover_layer(&self) -> El {
        let mut layer = cover()
            .linear_gradient(
                std::f32::consts::PI,
                [
                    (0.4, Color::BLACK.alpha(0.0)),
                    (1.0, Color::BLACK.alpha(0.8)),
                ],
            )
            .group_opacity(0.0, 1.0);
        if self.info.is_some() {
            // The card itself plays: say so in the middle.
            layer = layer.center().child(
                el().size(54.0, 54.0)
                    .radius(27.0)
                    .center()
                    .fill(Color::BLACK.alpha(0.5))
                    .border(2.0, Color::WHITE)
                    .child(icon(&icons::PLAY, 24.0).color(Color::WHITE)),
            );
        }
        let quick = |action: &Option<Action>, glyph, help: &str, variant| {
            action.clone().map(|action| {
                Button::icon(glyph, help)
                    .variant(variant)
                    .size(Size::Medium)
                    .round()
                    .on_press(move |s: &mut App, tx| s.act(action.clone(), tx))
                    .build()
            })
        };
        if let Some(play) = quick(&self.play, &icons::PLAY, "Play", Variant::Light) {
            layer = layer.child(play.pin(Some(10.0), None, None, Some(12.0)));
        }
        if let Some(info) = quick(&self.info, &icons::INFO, "Details", Variant::Glass) {
            layer = layer.child(info.pin(None, None, Some(10.0), Some(12.0)));
        }
        layer
    }
}

/// Stands in for a missing or broken poster: the title on a flat tile.
fn stand_in(title: &str) -> El {
    el().w_full().h_full().center().padding(12.0).child(
        text(title)
            .font(ty::SMALL_STRONG)
            .color(t().muted)
            .w_full()
            .wrap_anywhere()
            .max_lines(6)
            .align(rune::Align::Center),
    )
}

/// A strip's silhouette while its titles load: the same height as real
/// cards, so nothing moves when they arrive.
pub fn skeleton_strip() -> El {
    let mut cards = rune::row().gap(14.0).px(super::theme::space::PAGE);
    for _ in 0..10 {
        cards = cards.child(
            col()
                .w(CARD_W)
                .gap(8.0)
                .child(skeleton(Some(CARD_W), POSTER_H))
                .child(skeleton(Some(CARD_W * 0.75), 14.0))
                .child(skeleton(Some(CARD_W * 0.45), 12.0)),
        );
    }
    el().key("loading")
        .w_full()
        .h(POSTER_H + 54.0)
        .clip_rounded(0.1)
        .child(cards)
}
