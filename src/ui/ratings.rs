//! Rating badges: an IMDb pill, a Rotten Tomatoes tomato and a Metacritic
//! square, each left out when its score is missing.

use rune::color::hex;
use rune::{Color, El, el, row, text};

use super::theme::ty;
use crate::ratings::Ratings;

/// The Tomatometer's "fresh" line.
const FRESH_FROM: u8 = 60;

fn c(s: &str) -> Color {
    hex(s).unwrap_or(Color::WHITE)
}

/// Badges for fetched scores, using Cinemeta's IMDb score when there's no
/// fetched one. `None` when there's no score at all.
pub fn badges(r: Option<&Ratings>, fallback_imdb: Option<&str>, small: bool) -> Option<El> {
    let imdb = r
        .and_then(|r| r.imdb.clone())
        .or_else(|| fallback_imdb.map(str::to_string))
        .filter(|s| !s.trim().is_empty() && s.trim() != "N/A");
    let tomatoes = r.and_then(|r| r.rotten_tomatoes);
    let meta = r.and_then(|r| r.metacritic);
    if imdb.is_none() && tomatoes.is_none() && meta.is_none() {
        return None;
    }
    let font = if small { ty::TINY } else { ty::SMALL_STRONG };
    let s = |big: f32| if small { big * 0.8 } else { big };
    let mut out = row().gap(s(14.0)).center_y();

    if let Some(score) = imdb {
        out = out.child(
            row()
                .gap(s(6.0))
                .center_y()
                .child(
                    el().px(s(4.0))
                        .py(1.0)
                        .radius(3.0)
                        .fill(c("#f5c518"))
                        .child(
                            text("IMDb")
                                .font(rune::TextStyle {
                                    weight: 850.0,
                                    ..font
                                })
                                .color(Color::BLACK),
                        ),
                )
                .child(text(score).font(font).color(Color::WHITE)),
        );
    }
    if let Some(percent) = tomatoes {
        let d = s(15.0);
        let tomato = if percent >= FRESH_FROM {
            // A red tomato with a little green stem on top.
            el().size(d, d)
                .child(el().size(d, d).radius(d / 2.0).fill(c("#fa320a")))
                .child(
                    el().absolute(d * 0.3, -1.0)
                        .size(d * 0.4, d * 0.25)
                        .radius(2.0)
                        .fill(c("#00912d")),
                )
        } else {
            // A green splat: a blob with a couple of drops.
            let green = c("#0ac855");
            el().size(d, d)
                .child(
                    el().absolute(d * 0.1, d * 0.15)
                        .size(d * 0.8, d * 0.8)
                        .radius(d * 0.4)
                        .fill(green),
                )
                .child(
                    el().absolute(d * 0.7, 0.0)
                        .size(d * 0.3, d * 0.3)
                        .radius(d * 0.15)
                        .fill(green),
                )
                .child(
                    el().absolute(0.0, d * 0.75)
                        .size(d * 0.25, d * 0.25)
                        .radius(d * 0.125)
                        .fill(green),
                )
        };
        out = out.child(
            row()
                .gap(s(5.0))
                .center_y()
                .child(tomato)
                .child(text(format!("{percent}%")).font(font).color(Color::WHITE)),
        );
    }
    if let Some(score) = meta {
        let (fill, ink) = match score {
            61.. => (c("#66cc33"), Color::BLACK),
            40..=60 => (c("#ffcc33"), Color::BLACK),
            _ => (c("#ff0000"), Color::WHITE),
        };
        out = out.child(
            el().min_w(s(22.0))
                .h(s(20.0))
                .px(2.0)
                .center()
                .radius(2.0)
                .fill(fill)
                .child(text(score.to_string()).font(font).color(ink)),
        );
    }
    Some(out)
}
