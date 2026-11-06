//! Vertical scrolling for pages and sheets: a scroll viewport with a scroll
//! bar. The bar runs on live values, so scrolling never runs the view.

use rune::{Color, El, ScrollPos, ScrollSpec, col};
use rune_kit::{ScrollBar, ScrollBarStyle};

use super::theme::t;

/// A vertical scroll viewport keyed `key` over `content`, with a scroll
/// bar. `also` hears every scroll position.
pub fn scroll_area(key: &str, content: El, also: impl Fn(ScrollPos) + 'static) -> El {
    let th = t();
    col()
        .key(key)
        .w_full()
        .h_full()
        .scroll(ScrollSpec {
            stick_bottom: false,
        })
        .on_scroll_offset(also)
        .child(content)
        // Brighter than the kit's thumb, which is made for light panels.
        .scroll_bar_styled(
            ScrollBarStyle::from_kit(&th.kit)
                .thumb(Color::WHITE.alpha(0.3))
                .thumb_hover(Color::WHITE.alpha(0.55)),
        )
}
