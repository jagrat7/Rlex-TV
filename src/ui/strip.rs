//! A strip: a horizontally scrolling row of cards, as on every browse page.
//!
//! A catalog has 50 titles or more, and a page has several strips, so a strip
//! builds only the cards within reach of where it's scrolled: what shows,
//! and a screen more. Scrolling toward the end of what's built asks for a
//! rebuild that reaches further. The cards past it are one empty box of
//! their width, so the strip scrolls as far as it would with all of them.
//!
//! Arrows at its edges, shown on hover, page it a screenful at a time. They
//! fade out at the ends through live values, without running the view.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use rune::{El, Live, ScrollPos, Tx, el, icon, row};

use super::theme::{motion, space, t};
use super::{App, Bg, icons};

/// Where a strip is scrolled, and how far its cards are built.
struct Track {
    offset: Cell<f32>,
    viewport: Cell<f32>,
    built: Cell<f32>,
    back: Live<f32>,
    forward: Live<f32>,
}

/// Every strip's track, by key, and whether one needs to build further.
pub struct Strips {
    tracks: RefCell<HashMap<String, Rc<Track>>>,
    dirty: Rc<Cell<bool>>,
    bg: Bg,
}

impl Strips {
    pub fn new(bg: Bg) -> Self {
        Self {
            tracks: RefCell::default(),
            dirty: Rc::default(),
            bg,
        }
    }

    fn track(&self, key: &str) -> Rc<Track> {
        self.tracks
            .borrow_mut()
            .entry(key.to_string())
            .or_insert_with(|| {
                Rc::new(Track {
                    offset: Cell::new(0.0),
                    viewport: Cell::new(0.0),
                    built: Cell::new(0.0),
                    back: Live::new(0.0),
                    forward: Live::new(1.0),
                })
            })
            .clone()
    }

    /// Whether a strip scrolled near the end of its built cards since the
    /// last call; `poll` returns it so the view runs again.
    pub fn take_dirty(&self) -> bool {
        self.dirty.replace(false)
    }

    /// Scrolls strip `key` a screenful back or forward.
    pub fn page(&self, key: &str, forward: bool, tx: &mut Tx) {
        let track = self.track(key);
        let step = (track.viewport.get() - CARD_STEP).max(CARD_STEP);
        let to = track.offset.get() + if forward { step } else { -step };
        tx.scroll_to_with(scroll_key(key), to.max(0.0), 0.0, motion::GLIDE);
    }
}

/// A card's width plus the gap after it, for paging and reach.
const CARD_STEP: f32 = super::poster::CARD_W + GAP;
const GAP: f32 = 14.0;

fn scroll_key(key: &str) -> String {
    format!("{key}/scroll")
}

/// A strip of `count` cards `width` wide, built by `card(i)`. `height` is
/// the cards' height, so arrows centre on the posters.
pub fn strip(s: &App, key: &str, count: usize, poster_h: f32, card: impl Fn(usize) -> El) -> El {
    let track = s.strips.track(key);
    let total = count as f32 * CARD_STEP;
    // Before the first layout, assume a wide window.
    let viewport = match track.viewport.get() {
        0.0 => 1920.0,
        v => v,
    };
    let reach = (track.offset.get() + 2.0 * viewport).max(track.built.get());
    let built = ((reach / CARD_STEP).ceil() as usize).min(count);
    track.built.set(built as f32 * CARD_STEP);

    let mut cards = row().gap(GAP).px(space::PAGE).items_start();
    for i in 0..built {
        cards = cards.child(card(i));
    }
    if built < count {
        cards = cards.child(
            el().key("rest")
                .w(total - built as f32 * CARD_STEP - GAP)
                .h(1.0),
        );
    }

    let watch = {
        let (track, dirty, bg) = (track.clone(), s.strips.dirty.clone(), s.strips.bg.clone());
        move |at: ScrollPos| {
            let ([x, _], [w, _], [max, _]) = (at.offset, at.viewport, at.max);
            track.offset.set(x);
            track.viewport.set(w);
            track.back.set(if x > 1.0 { 1.0 } else { 0.0 });
            track.forward.set(if x < max - 1.0 { 1.0 } else { 0.0 });
            let wanted = x + 1.5 * w;
            if wanted > track.built.get() && track.built.get() < total && !dirty.get() {
                dirty.set(true);
                bg.wake();
            }
        }
    };

    el().key(key)
        .w_full()
        .hover_group()
        .child(
            el().key(scroll_key(key))
                .w_full()
                .scroll_x()
                .scroll_edge_fade(24.0)
                .on_scroll_offset(watch)
                .child(cards.pb(6.0)),
        )
        .child(arrow(key, false, poster_h, &track.back))
        .child(arrow(key, true, poster_h, &track.forward))
}

/// The edge arrow that pages a strip; shown while the strip is hovered and
/// there's more that way.
fn arrow(strip: &str, forward: bool, poster_h: f32, shown: &Live<f32>) -> El {
    let th = t();
    let edge = if forward {
        (None, Some(0.0))
    } else {
        (Some(0.0), None)
    };
    let fade = [(0.0, th.bg.alpha(0.0)), (1.0, th.bg.alpha(0.88))];
    let stops = if forward {
        fade
    } else {
        [(0.0, fade[1].1), (1.0, fade[0].1)]
    };
    let key = strip.to_string();
    el().key(if forward { "next" } else { "prev" })
        .pin(edge.0, Some(0.0), edge.1, None)
        .w(64.0)
        .h(poster_h)
        .center()
        .linear_gradient(std::f32::consts::FRAC_PI_2, stops)
        .live_opacity(shown)
        .group_opacity(0.0, 1.0)
        .cursor(rune::Cursor::Pointer)
        .role("button")
        .label(if forward { "Next" } else { "Previous" })
        .on_press_tx(move |s: &mut App, tx| s.strips.page(&key, forward, tx))
        .child(
            el().size(40.0, 40.0)
                .radius(20.0)
                .center()
                .fill(rune::Color::BLACK.alpha(0.6))
                .border(1.0, rune::Color::WHITE.alpha(0.45))
                .hover(rune::Color::WHITE.alpha(0.18))
                .child(
                    icon(
                        if forward {
                            &icons::CHEVRON_RIGHT
                        } else {
                            &icons::CHEVRON_LEFT
                        },
                        22.0,
                    )
                    .color(rune::Color::WHITE),
                ),
        )
}
