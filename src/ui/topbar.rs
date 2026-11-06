//! The top bar, laid over the page: the brand, the pages, search and
//! Settings. Clear over a page's artwork, it frosts over as the page scrolls
//! under it (a live value the page sets, so scrolling never runs the view).

use std::f32::consts::PI;

use rune::{Cursor, Cx, El, el, row, spacer, text};
use rune_kit::TextInput;

use super::theme::{motion, space, t, ty};
use super::widgets::{Button, Variant, cover};
use super::{Action, App, Page, icons, on, search};

pub const BAR_H: f32 = 64.0;

pub fn bar(s: &App, cx: &mut Cx<App>) -> El {
    let th = t();
    let gradient = cover().linear_gradient(PI, [(0.0, th.bg.alpha(0.92)), (1.0, th.bg.alpha(0.0))]);
    let frost = cover()
        .fill(th.bg.alpha(0.8))
        .backdrop_blur(20.0)
        .live_opacity(&s.bar)
        .child(
            el().pin(Some(0.0), None, Some(0.0), Some(0.0))
                .h(1.0)
                .fill(th.line),
        );

    let mut bar = row()
        .key("topbar")
        .pin(Some(0.0), Some(0.0), Some(0.0), None)
        .h(BAR_H)
        .px(space::PAGE)
        .gap(30.0)
        .center_y()
        .child(gradient)
        .child(frost);
    if let Some(parked) = s.nav.parked.last() {
        bar = bar.child(
            Button::new(format!("Back to {}", parked.target.name))
                .with_icon(&icons::ARROW_LEFT)
                .variant(Variant::Glass)
                .on_press(on(Action::Back))
                .build(),
        );
    }
    bar.child(brand(s))
        .child(nav(s, cx))
        .child(spacer())
        .child({
            let field = TextInput::new(&s.search.edit)
                .placeholder("Titles, people, genres")
                .label("Search")
                .leading_icon(&icons::SEARCH)
                .width(Some(280.0));
            // Clearing is offered once there's something to clear.
            if s.search.edit.text.is_empty() {
                field
            } else {
                field.clear_button()
            }
            .on_edit(|s: &mut App| &mut s.search.edit)
            .on_change(|s: &mut App, tx| s.search_changed(tx))
            // Esc clears the search and goes back to where you were.
            .on_key(|s: &mut App, ev, tx| {
                if ev.key != rune::Key::Escape {
                    return false;
                }
                tx.blur();
                s.search.edit.clear();
                s.search_changed(tx);
                true
            })
            .on_submit(|s: &mut App, _| s.search.run_now(&s.core))
            .build(&th.kit)
            .key(search::FIELD)
        })
        .child({
            let here = s.nav.page == Page::Settings && s.nav.details.is_empty();
            let gear = Button::icon(&icons::SETTINGS, "Settings")
                .variant(if here { Variant::Glass } else { Variant::Ghost })
                .on_press(on(Action::Go(Page::Settings)));
            gear.build()
        })
}

fn brand(s: &App) -> El {
    row()
        .key("brand")
        .gap(10.0)
        .center_y()
        .cursor(Cursor::Pointer)
        .role("link")
        .label("Rlex, home")
        .on_press_tx(on(Action::Go(Page::Home)))
        .child(el().size(30.0, 30.0).pixels(&s.images.logo))
        .child(
            text("Rlex")
                .font(rune::TextStyle::new(21.0, 800.0).tracking(-0.4))
                .color(t().text),
        )
}

/// The page links, with an underline that slides to the one showing.
fn nav(s: &App, cx: &mut Cx<App>) -> El {
    let th = t();
    let current = s.nav.page;
    let mut links = row()
        .key("nav")
        .gap(24.0)
        .h(BAR_H)
        .center_y()
        .role("tablist");
    for page in Page::NAV {
        let active = page == current;
        links = links.child(
            text(page.label())
                .key(link_key(page))
                .font(if active {
                    ty::LABEL
                } else {
                    rune::TextStyle {
                        weight: 500.0,
                        ..ty::LABEL
                    }
                })
                .color(if active { th.text } else { th.muted })
                .group_color(th.text)
                .hover_group()
                .cursor(Cursor::Pointer)
                .role("tab")
                .checked(active)
                .focusable(true)
                .on_press_tx(on(Action::Go(page))),
        );
    }
    // The current link's place along the row, from the last layout.
    let link = Page::NAV
        .contains(&current)
        .then(|| cx.layout(&link_key(current)))
        .flatten();
    let (x, w, shown) = match (link, cx.layout("nav")) {
        (Some([x, _, w, _]), Some([row_x, ..])) => (x - row_x, w, 1.0),
        _ => (0.0, 0.0, 0.0),
    };
    links.child(
        el().key("underline")
            .absolute(x, BAR_H / 2.0 + 14.0)
            .size(w, 2.5)
            .radius(1.25)
            .fill(th.accent)
            .opacity(shown)
            .motion(motion::PANEL)
            .a11y_hidden(),
    )
}

fn link_key(page: Page) -> String {
    format!("nav-{}", page.key())
}
