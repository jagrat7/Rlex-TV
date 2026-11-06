//! Toasts: short notes about background work (a source added, a player that
//! didn't start) at the bottom of the window. Each lasts a few seconds, or
//! until it's closed.

use std::time::Duration;

use rune::{Cursor, El, Transition, Tx, col, el, icon, row, text};

use super::theme::{float_shadow, t, ty};
use super::widgets::Tone;
use super::{App, icons};

/// Seconds a toast stays up.
const LIFE: f64 = 5.0;

#[derive(Default)]
pub struct Toasts {
    list: Vec<Toast>,
}

struct Toast {
    id: u64,
    tone: Tone,
    text: String,
    /// When it goes, on the poll clock; set the first frame it shows.
    until: Option<f64>,
}

impl Toasts {
    /// Times new toasts and drops expired ones; `true` when one went.
    pub fn tick(&mut self, clock: f64, tx: &mut Tx) -> bool {
        let before = self.list.len();
        for toast in &mut self.list {
            toast.until.get_or_insert(clock + LIFE);
        }
        self.list
            .retain(|toast| toast.until.is_some_and(|until| until > clock));
        if let Some(next) = self
            .list
            .iter()
            .filter_map(|toast| toast.until)
            .reduce(f64::min)
        {
            tx.wake_after(Duration::from_secs_f64((next - clock).max(0.05)));
        }
        self.list.len() != before
    }
}

impl App {
    pub fn toast_error(&mut self, text: impl Into<String>) {
        self.toast(Tone::Danger, text.into());
    }

    pub fn toast_info(&mut self, text: impl Into<String>) {
        self.toast(Tone::Info, text.into());
    }

    pub fn toast_success(&mut self, text: impl Into<String>) {
        self.toast(Tone::Success, text.into());
    }

    fn toast(&mut self, tone: Tone, text: String) {
        let id = self.next_id();
        let list = &mut self.toasts.list;
        // The same note twice in a row stays one toast, timed afresh.
        list.retain(|toast| toast.text != text);
        list.push(Toast {
            id,
            tone,
            text,
            until: None,
        });
        if list.len() > 4 {
            list.remove(0);
        }
    }
}

/// The toasts, newest at the bottom.
pub fn stack(s: &App) -> El {
    let th = t();
    let mut stack = col()
        .key("toasts")
        .pin(None, None, Some(24.0), Some(24.0))
        .w(380.0)
        .gap(10.0)
        .items_end();
    for toast in &s.toasts.list {
        let (glyph, color) = match toast.tone {
            Tone::Danger => (&icons::ALERT, th.danger),
            Tone::Success => (&icons::CIRCLE_CHECK, th.success),
            _ => (&icons::INFO, th.info),
        };
        let id = toast.id;
        stack = stack.child(
            row()
                .key(format!("toast-{id}"))
                .w_full()
                .gap(12.0)
                .items_start()
                .padding(14.0)
                .radius(12.0)
                .fill(th.raised2)
                .border(1.0, th.line)
                .box_shadow(float_shadow())
                .role("status")
                .live_region(rune::LiveRegion::Polite)
                .transition(Transition::offset(0.0, 16.0).combined(Transition::opacity()))
                .child(icon(glyph, 18.0).color(color).mt(1.0))
                .child(
                    text(toast.text.clone())
                        .font(ty::SMALL)
                        .color(th.text)
                        .wrap()
                        .grow(1.0)
                        .shrink(),
                )
                .child(
                    el().key("close")
                        .size(20.0, 20.0)
                        .radius(5.0)
                        .center()
                        .hover(th.hover)
                        .cursor(Cursor::Pointer)
                        .role("button")
                        .label("Dismiss")
                        .on_press(move |s: &mut App| s.toasts.list.retain(|toast| toast.id != id))
                        .child(icon(&icons::CLOSE, 14.0).color(th.muted)),
                ),
        );
    }
    stack
}
