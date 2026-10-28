//! Small pieces every page shares: loading states, headings, empty states,
//! the folder picker, and number formatting.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::scroll::ScrollableElement;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt, h_flex, v_flex};
use gpui_kit::*;

use crate::state::AppState;

/// A slow value a page is waiting on.
pub enum Load<T> {
    Loading,
    Ready(T),
    Failed(String),
}

impl<T> Load<T> {
    pub fn from_result(result: anyhow::Result<T>) -> Self {
        match result {
            Ok(value) => Self::Ready(value),
            Err(err) => Self::Failed(format!("{err:#}")),
        }
    }
}

/// How long typing pauses before a search runs on its own.
pub const DEBOUNCE: Duration = Duration::from_millis(450);

/// An element id built from a dynamic string, for rows and buttons in lists.
pub fn eid(name: impl Into<String>) -> ElementId {
    ElementId::Name(SharedString::from(name.into()))
}

/// The page body: padded, vertically scrolling, with a scrollbar.
pub fn page(id: &'static str) -> gpui_kit::component::scroll::Scrollable<Div> {
    v_flex().gap_6().p_6().overflow_y_scrollbar().id(id)
}

pub fn page_title(text: impl Into<SharedString>) -> Div {
    div().text_2xl().font_bold().child(text.into())
}

pub fn heading(text: impl Into<SharedString>) -> Div {
    div().text_lg().font_semibold().child(text.into())
}

/// A heading with the content underneath it.
pub fn section(text: impl Into<SharedString>, body: impl IntoElement) -> Div {
    v_flex().gap_3().child(heading(text)).child(body)
}

pub fn muted(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

pub fn error_text(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_sm()
        .text_color(cx.theme().danger)
        .child(text.into())
}

/// A spinner with a short explanation next to it.
pub fn busy(text: impl Into<SharedString>, cx: &App) -> Div {
    h_flex()
        .gap_2()
        .items_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(Spinner::new().small())
        .child(text.into())
}

/// A raised panel for grouped content.
pub fn card(cx: &App) -> Div {
    div()
        .rounded(cx.theme().radius_lg)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary.opacity(0.4))
}

/// A centered message for pages with nothing to show yet.
pub fn empty_state(
    icon: IconName,
    title: impl Into<SharedString>,
    body: impl Into<SharedString>,
    cx: &App,
) -> Div {
    v_flex()
        .w_full()
        .py_16()
        .items_center()
        .gap_3()
        .text_center()
        .child(
            Icon::new(icon)
                .size(px(40.))
                .text_color(cx.theme().muted_foreground),
        )
        .child(div().text_lg().font_semibold().child(title.into()))
        .child(
            div()
                .max_w(px(420.))
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(body.into()),
        )
}

/// Placeholder lines while text loads.
pub fn skeleton_lines(count: usize, cx: &App) -> Div {
    v_flex().gap_2().w_full().children((0..count).map(|i| {
        let line = Skeleton::new().h_4().rounded(cx.theme().radius);
        if i + 1 == count {
            line.w_2_3()
        } else {
            line.w_full()
        }
    }))
}

/// Opens the system folder picker and adds every chosen folder to the library.
pub fn pick_folders(state: Entity<AppState>, cx: &mut App) {
    let picker = cx.prompt_for_paths(PathPromptOptions {
        files: false,
        directories: true,
        multiple: true,
        prompt: Some("Add to library".into()),
    });
    cx.spawn(async move |cx| {
        let Ok(Ok(Some(paths))) = picker.await else {
            return;
        };
        state.update(cx, |state, cx| {
            for path in paths {
                state.add_folder(path, cx);
            }
        });
    })
    .detach();
}

/// `12:34` or `1:02:03`.
pub fn fmt_clock(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// `1.4 GiB`, `312 MiB`, `9.0 KiB`.
pub fn fmt_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Bytes per second as `2.3 MiB/s`.
pub fn fmt_speed(bytes_per_second: f64) -> String {
    format!("{:.1} MiB/s", bytes_per_second.max(0.0) / (1024.0 * 1024.0))
}

/// The first ten characters of an ISO date, or the string as it came.
pub fn fmt_date(iso: &str) -> String {
    iso.get(..10).unwrap_or(iso).to_string()
}
