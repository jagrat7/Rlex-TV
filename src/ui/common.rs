//! Small pieces every page shares: loading states, headings, empty states,
//! the folder picker, and number formatting.

use std::time::Duration;

use gpui_kit::assets::IconName;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{ActiveTheme, Icon, Sizable, StyledExt, WindowExt, h_flex, v_flex};
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

/// The top bar's height. It floats over the page, so pages start below it
/// unless they open with full-bleed artwork.
pub const TOP_BAR_HEIGHT: f32 = 60.;

/// The page body, padded and clear of the top bar. The shell scrolls it.
pub fn page(id: &'static str) -> Stateful<Div> {
    v_flex()
        .id(id)
        .w_full()
        .gap_6()
        .p_6()
        .pt(px(TOP_BAR_HEIGHT + 16.))
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

/// The big buttons over hero artwork: white for the main action,
/// translucent for the rest.
pub fn hero_button(
    id: impl Into<ElementId>,
    icon: IconName,
    label: impl Into<SharedString>,
    primary: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let (bg, hover, fg) = if primary {
        (white(), white().alpha(0.8), black())
    } else {
        (hsla(0., 0., 0.55, 0.45), hsla(0., 0., 0.55, 0.3), white())
    };
    h_flex()
        .id(id.into())
        .h(px(42.))
        .px_6()
        .gap_2()
        .items_center()
        .rounded(px(6.))
        .bg(bg)
        .text_color(fg)
        .font_semibold()
        .cursor_pointer()
        .hover(move |this| this.bg(hover))
        .child(Icon::new(icon).size_5().text_color(fg))
        .child(label.into())
        .on_click(on_click)
}

/// Says why a second Play did nothing: one player is still starting.
pub fn still_starting(window: &mut Window, cx: &mut App) {
    window.push_notification(
        Notification::info("Still starting the last one. Torrents can take a few seconds."),
        cx,
    );
}

/// Waits for a player to start, then runs `done` on the view and reports a
/// failure as a toast.
pub fn await_player<V: 'static>(
    task: Task<anyhow::Result<()>>,
    window: &mut Window,
    cx: &mut Context<V>,
    done: impl FnOnce(&mut V) + 'static,
) {
    cx.spawn_in(window, async move |this, cx| {
        let result = task.await;
        this.update_in(cx, |this, window, cx| {
            done(this);
            if let Err(err) = result {
                window
                    .push_notification(Notification::error(format!("Couldn't play: {err:#}")), cx);
            }
            cx.notify();
        })
        .ok();
    })
    .detach();
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

/// An ISO date as `Jan 20, 2008`, or the string as it came if it isn't one.
pub fn fmt_date(iso: &str) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let date = iso.get(..10).unwrap_or(iso);
    let mut parts = date.split('-').map(str::parse::<u32>);
    match (parts.next(), parts.next(), parts.next()) {
        (Some(Ok(year)), Some(Ok(month @ 1..=12)), Some(Ok(day @ 1..=31))) => {
            format!("{} {day}, {year}", MONTHS[month as usize - 1])
        }
        _ => date.to_string(),
    }
}

/// Minutes in a runtime such as `142 min`, `2h 22min` or `1 hr 56 min`.
pub fn runtime_minutes(runtime: &str) -> Option<u32> {
    let mut total = 0;
    let mut chars = runtime.chars().peekable();
    while let Some(c) = chars.next() {
        let Some(digit) = c.to_digit(10) else {
            continue;
        };
        let mut n = digit;
        while let Some(d) = chars.peek().and_then(|c| c.to_digit(10)) {
            n = n.checked_mul(10)?.checked_add(d)?;
            chars.next();
        }
        while chars.next_if(|c| c.is_whitespace()).is_some() {}
        // A number followed by `h` counts hours; anything else, minutes.
        let hours = chars.peek().is_some_and(|c| c.eq_ignore_ascii_case(&'h'));
        total += if hours { n.checked_mul(60)? } else { n };
    }
    (total > 0).then_some(total)
}

/// `S1 E3` from an episode label like `S1 E3 · Pilot`.
pub fn short_episode(label: &str) -> String {
    label.split(" · ").next().unwrap_or(label).to_string()
}

/// Minutes left of a video, rounded up, as `1h 59m left`.
pub fn fmt_time_left(position_secs: f64, duration_secs: f64) -> String {
    let left = ((duration_secs - position_secs) / 60.0).ceil().max(1.0) as u32;
    format!("{} left", fmt_minutes(left))
}

/// `49m`, or `2h 22m` once it's over an hour.
pub fn fmt_minutes(minutes: u32) -> String {
    let (hours, rest) = (minutes / 60, minutes % 60);
    if minutes <= 60 {
        format!("{minutes}m")
    } else if rest == 0 {
        format!("{hours}h")
    } else {
        format!("{hours}h {rest}m")
    }
}

/// A Cinemeta runtime in the `2h 22m` style, or as it came if it can't be read.
pub fn fmt_runtime(runtime: &str) -> String {
    runtime_minutes(runtime).map_or_else(|| runtime.to_string(), fmt_minutes)
}

#[cfg(test)]
mod tests {
    use super::{fmt_date, fmt_runtime, fmt_time_left};

    #[test]
    fn formats_runtimes() {
        assert_eq!(fmt_runtime("49 min"), "49m");
        assert_eq!(fmt_runtime("60 min"), "60m");
        assert_eq!(fmt_runtime("61 min"), "1h 1m");
        assert_eq!(fmt_runtime("142 min"), "2h 22m");
        assert_eq!(fmt_runtime("120 min"), "2h");
        assert_eq!(fmt_runtime("2h 22min"), "2h 22m");
        assert_eq!(fmt_runtime("1 hr 56 min"), "1h 56m");
        assert_eq!(fmt_runtime("N/A"), "N/A");
    }

    #[test]
    fn formats_time_left() {
        assert_eq!(fmt_time_left(3000.0, 10140.0), "1h 59m left");
        assert_eq!(fmt_time_left(1200.0, 2900.0), "29m left");
        assert_eq!(fmt_time_left(2899.0, 2900.0), "1m left");
    }

    #[test]
    fn formats_dates() {
        assert_eq!(fmt_date("2008-01-20T05:00:00.000Z"), "Jan 20, 2008");
        assert_eq!(fmt_date("2008"), "2008");
    }
}
