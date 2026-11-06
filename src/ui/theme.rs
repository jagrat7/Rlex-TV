//! Rlex's look: a warm near-black with the logo's rust as the accent, Inter
//! for text, and one spacing and type scale for every page.

use std::sync::OnceLock;

use rune::color::hex;
use rune::{Color, Shadow, Spring, TextStyle};
use rune_kit::{Kit, PaletteSeed};

pub struct Theme {
    pub bg: Color,
    /// Cards, rows and panels on the page.
    pub raised: Color,
    /// Fields, chips and skeletons on a raised surface.
    pub raised2: Color,
    pub line: Color,
    pub hover: Color,
    pub text: Color,
    pub muted: Color,
    pub faint: Color,
    pub accent: Color,
    pub accent_hover: Color,
    pub on_accent: Color,
    pub success: Color,
    pub warning: Color,
    pub danger: Color,
    pub info: Color,
    pub kit: Kit,
}

pub fn t() -> &'static Theme {
    static THEME: OnceLock<Theme> = OnceLock::new();
    THEME.get_or_init(|| {
        let c = |s: &str| hex(s).expect("theme colours are valid hex");
        let (bg, text, accent) = (c("#0d0c0b"), c("#f4f0ea"), c("#d2692e"));
        let mut kit = Kit::from_palette(PaletteSeed {
            bg,
            fg: text,
            accent,
        });
        kit.radius.sm = 6.0;
        kit.radius.md = 8.0;
        Theme {
            bg,
            raised: c("#181614"),
            raised2: c("#24211e"),
            line: Color::WHITE.alpha(0.09),
            hover: Color::WHITE.alpha(0.07),
            text,
            muted: c("#a8a097"),
            faint: c("#6f6861"),
            accent,
            accent_hover: c("#e47b3d"),
            on_accent: c("#fffaf5"),
            success: c("#4cc38a"),
            warning: c("#e8ab42"),
            danger: c("#ec5b52"),
            info: c("#64aee8"),
            kit,
        }
    })
}

/// Spacing, in points.
pub mod space {
    /// The page's side margin.
    pub const PAGE: f32 = 32.0;
    /// Between sections of a page.
    pub const SECTION: f32 = 36.0;
}

/// Text styles, largest first.
pub mod ty {
    use super::TextStyle;
    pub const DISPLAY: TextStyle = TextStyle::new(46.0, 800.0).lh(52.0).tracking(-0.6);
    pub const TITLE: TextStyle = TextStyle::new(28.0, 750.0).lh(34.0).tracking(-0.3);
    pub const HEADING: TextStyle = TextStyle::new(19.0, 650.0).lh(26.0);
    pub const SUBHEAD: TextStyle = TextStyle::new(15.0, 600.0).lh(21.0);
    pub const BODY: TextStyle = TextStyle::new(14.0, 400.0).lh(21.0);
    pub const LABEL: TextStyle = TextStyle::new(14.0, 600.0).lh(20.0);
    pub const SMALL: TextStyle = TextStyle::new(13.0, 400.0).lh(18.0);
    pub const SMALL_STRONG: TextStyle = TextStyle::new(13.0, 600.0).lh(18.0);
    pub const CAPTION: TextStyle = TextStyle::new(12.0, 500.0).lh(16.0);
    pub const TINY: TextStyle = TextStyle::new(11.0, 650.0).lh(14.0);
    pub const MONO: TextStyle = TextStyle::new(12.5, 400.0).lh(18.0).mono();
}

/// Springs.
pub mod motion {
    use super::Spring;
    /// Hover, press and small state changes.
    pub const QUICK: Spring = Spring::new(0.22, 0.0);
    /// Panels and pages coming and going.
    pub const PANEL: Spring = Spring::new(0.38, 0.06);
    /// A row of posters paging along.
    pub const GLIDE: Spring = Spring::new(0.55, 0.0);
}

/// A soft drop shadow for things that float.
pub fn float_shadow() -> Shadow {
    Shadow {
        x: 0.0,
        y: 18.0,
        blur: 48.0,
        spread: 0.0,
        color: Color::BLACK.alpha(0.55),
        inset: false,
    }
}
