//! The pieces every page shares: buttons, tags, progress bars, spinners,
//! skeletons, headings and empty states.

use rune::{Color, Cursor, El, Icon, LoopSpec, Tx, col, el, icon, row, text};

use super::App;
use super::icons;
use super::theme::{t, ty};

#[derive(Clone, Copy, PartialEq)]
pub enum Variant {
    /// White on the artwork: the main action of a hero.
    Light,
    /// The brand colour: the main action anywhere else.
    Accent,
    /// Translucent, for second actions over artwork.
    Glass,
    /// A quiet bordered button.
    Outline,
    /// Text only until hovered.
    Ghost,
    Danger,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Size {
    Small,
    Medium,
    Large,
}

type Handler = Box<dyn Fn(&mut App, &mut Tx)>;

/// A button: an optional icon and label, a variant and a size. Busy swaps
/// the icon for a spinner; disabled dims it and drops its handler.
pub struct Button {
    label: Option<String>,
    icon: Option<&'static Icon>,
    variant: Variant,
    size: Size,
    busy: bool,
    disabled: bool,
    round: bool,
    help: Option<String>,
    key: Option<String>,
    ink: Option<Color>,
    on_press: Option<Handler>,
}

impl Button {
    pub fn new(label: impl Into<String>) -> Self {
        Self::bare(Some(label.into()), None)
    }

    /// An icon-only button; `help` names it for tooltips and screen readers.
    pub fn icon(icon: &'static Icon, help: impl Into<String>) -> Self {
        let mut button = Self::bare(None, Some(icon));
        button.help = Some(help.into());
        button
    }

    fn bare(label: Option<String>, icon: Option<&'static Icon>) -> Self {
        Self {
            label,
            icon,
            variant: Variant::Outline,
            size: Size::Medium,
            busy: false,
            disabled: false,
            round: false,
            help: None,
            key: None,
            ink: None,
            on_press: None,
        }
    }

    pub fn with_icon(mut self, icon: &'static Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn variant(mut self, variant: Variant) -> Self {
        self.variant = variant;
        self
    }
    pub fn size(mut self, size: Size) -> Self {
        self.size = size;
        self
    }
    pub fn busy(mut self, busy: bool) -> Self {
        self.busy = busy;
        self
    }
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
    /// A circle, for icon-only buttons floating over artwork.
    pub fn round(mut self) -> Self {
        self.round = true;
        self
    }
    pub fn help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }
    /// The icon and label colour, instead of the variant's.
    pub fn ink(mut self, ink: Color) -> Self {
        self.ink = Some(ink);
        self
    }
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
    pub fn on_press(mut self, f: impl Fn(&mut App, &mut Tx) + 'static) -> Self {
        self.on_press = Some(Box::new(f));
        self
    }

    pub fn build(self) -> El {
        let th = t();
        let (h, pad, font, glyph) = match self.size {
            Size::Small => (28.0, 10.0, ty::CAPTION, 14.0),
            Size::Medium => (34.0, 14.0, ty::SMALL_STRONG, 16.0),
            Size::Large => (46.0, 24.0, ty::SUBHEAD, 22.0),
        };
        let (fill, ink, border) = match self.variant {
            Variant::Light => (Color::WHITE, Color::BLACK, None),
            Variant::Accent => (th.accent, th.on_accent, None),
            Variant::Glass => (Color::WHITE.alpha(0.18), Color::WHITE, None),
            Variant::Outline => (Color::TRANSPARENT, th.text, Some(th.line.alpha(0.16))),
            Variant::Ghost => (Color::TRANSPARENT, th.text, None),
            Variant::Danger => (th.danger.alpha(0.16), th.danger, None),
        };
        let ink = self.ink.unwrap_or(ink);
        let hover = match self.variant {
            Variant::Light => Color::BLACK.alpha(0.14),
            Variant::Accent => Color::WHITE.alpha(0.14),
            _ => th.hover,
        };
        let icon_only = self.label.is_none();
        let mut b =
            row()
                .key(self.key.unwrap_or_else(|| {
                    self.label.clone().or(self.help.clone()).unwrap_or_default()
                }))
                .h(h)
                .gap(if self.size == Size::Large { 10.0 } else { 7.0 })
                .center()
                .radius(if self.round { h / 2.0 } else { 8.0 })
                .fill(fill)
                .role("button")
                .focusable(!self.disabled)
                .motion(super::theme::motion::QUICK);
        b = if icon_only { b.w(h) } else { b.px(pad) };
        if let Some(border) = border {
            b = b.border(1.0, border);
        }
        if self.busy {
            b = b.child(spinner(glyph, ink));
        } else if let Some(i) = self.icon {
            b = b.child(icon(i, glyph).color(ink));
        }
        if let Some(label) = &self.label {
            b = b
                .label(label.clone())
                .child(text(label.clone()).font(font).color(ink));
        }
        if let Some(help) = self.help {
            b = if icon_only { b.label(help.clone()) } else { b }.help(help);
        }
        match self.on_press {
            Some(f) if !self.disabled && !self.busy => b
                .hover(hover)
                .press(hover.alpha(hover.a * 1.6))
                .cursor(Cursor::Pointer)
                .on_press_tx(f),
            _ if self.disabled => b.disabled(true).dim(0.4),
            _ => b,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Tone {
    Neutral,
    Accent,
    Success,
    Warning,
    Danger,
    Info,
}

impl Tone {
    fn color(self) -> Color {
        let th = t();
        match self {
            Tone::Neutral => th.muted,
            Tone::Accent => th.accent_hover,
            Tone::Success => th.success,
            Tone::Warning => th.warning,
            Tone::Danger => th.danger,
            Tone::Info => th.info,
        }
    }
}

/// A small label in a tinted pill: "Torrent", "1080p", "Next up".
pub fn tag(label: impl Into<String>, tone: Tone) -> El {
    let c = tone.color();
    row()
        .h(20.0)
        .px(7.0)
        .center()
        .radius(5.0)
        .fill(c.alpha(0.16))
        .child(text(label).font(ty::TINY).color(c))
}

/// A ring that turns while something loads.
pub fn spinner(size: f32, color: Color) -> El {
    icon(&icons::LOADER, size)
        .color(color)
        .looping(LoopSpec::rotation(0.85))
}

/// A spinner with a short explanation beside it.
pub fn busy(label: impl Into<String>) -> El {
    row()
        .gap(10.0)
        .center_y()
        .child(spinner(16.0, t().muted))
        .child(text(label).font(ty::SMALL).color(t().muted))
}

/// A placeholder block that pulses while its content loads.
pub fn skeleton(w: Option<f32>, h: f32) -> El {
    let block = el()
        .h(h)
        .radius(6.0)
        .fill(t().raised2)
        .looping(LoopSpec::opacity(1.6, 0.0, 0.5, 1.0));
    match w {
        Some(w) => block.w(w),
        None => block.w_full(),
    }
}

/// A thin bar filled to `value` (0–1).
pub fn progress(value: f32, height: f32, track: Color, fill: Color) -> El {
    el().w_full()
        .h(height)
        .radius(height / 2.0)
        .fill(track)
        .child(
            el().h_full()
                .w_frac(value.clamp(0.0, 1.0))
                .radius(height / 2.0)
                .fill(fill),
        )
}

pub fn page_title(label: impl Into<String>) -> El {
    text(label).font(ty::TITLE).color(t().text)
}

pub fn heading(label: impl Into<String>) -> El {
    text(label).font(ty::HEADING).color(t().text)
}

pub fn muted(label: impl Into<String>) -> El {
    text(label).font(ty::SMALL).color(t().muted).wrap()
}

pub fn error_text(label: impl Into<String>) -> El {
    text(label).font(ty::SMALL).color(t().danger).wrap()
}

/// A heading with the content underneath it.
pub fn section(title: &str, body: El) -> El {
    col()
        .key(title)
        .w_full()
        .gap(14.0)
        .child(heading(title))
        .child(body)
}

/// An element stretched over its parent.
pub fn cover() -> El {
    el().pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
}

/// A raised, rounded panel for grouped rows.
pub fn panel() -> El {
    col()
        .w_full()
        .radius(12.0)
        .fill(t().raised)
        .border(1.0, t().line)
        .clip_rounded(12.0)
}

/// A panel of rows with a hairline between each.
pub fn ruled(rows: impl IntoIterator<Item = El>) -> El {
    let mut out = panel();
    for (i, r) in rows.into_iter().enumerate() {
        if i > 0 {
            out = out.child(el().key(format!("rule-{i}")).w_full().h(1.0).fill(t().line));
        }
        out = out.child(r);
    }
    out
}

/// A centred message for pages with nothing to show yet.
pub fn empty_state(glyph: &'static Icon, title: &str, body: &str) -> El {
    col()
        .key("empty")
        .w_full()
        .py(72.0)
        .gap(12.0)
        .items_center()
        .child(
            el().size(72.0, 72.0)
                .radius(36.0)
                .center()
                .fill(t().raised2)
                .child(icon(glyph, 32.0).color(t().muted)),
        )
        .child(text(title).font(ty::HEADING).color(t().text).mt(6.0))
        .child(
            text(body)
                .font(ty::BODY)
                .color(t().muted)
                .max_w(440.0)
                .wrap()
                .align(rune::Align::Center),
        )
}

/// Joins the parts that are there with a middle dot.
pub fn dotted<I: IntoIterator<Item = Option<String>>>(parts: I) -> String {
    parts
        .into_iter()
        .flatten()
        .filter(|p| !p.is_empty())
        .collect::<Vec<_>>()
        .join("  ·  ")
}
