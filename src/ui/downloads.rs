//! Downloads: the torrent engine's transfers. While the page shows, `poll`
//! reads the engine's numbers once a second, and the view runs only when
//! they changed.

use std::time::Duration;

use rune::{Color, El, Tx, col, el, row, text};

use super::format::{fmt_bytes, fmt_speed};
use super::theme::{float_shadow, space, t, ty};
use super::topbar::BAR_H;
use super::widgets::{
    Button, Size, Tone, Variant, busy, empty_state, error_text, page_title, panel, progress, tag,
};
use super::{App, Page, icons, scroller};
use crate::state::EngineStatus;
use crate::torrent::TorrentInfo;

#[derive(Default)]
pub struct Downloads {
    list: Vec<TorrentInfo>,
    /// When to read the engine again, on the poll clock.
    next: f64,
    /// A download waiting on "keep or delete its files?".
    pub(super) confirm: Option<(usize, String)>,
}

impl App {
    /// While Downloads shows, reads the engine's numbers once a second;
    /// `true` when they changed.
    pub fn refresh_downloads(&mut self, clock: f64, tx: &mut Tx) -> bool {
        if self.nav.page != Page::Downloads || !self.nav.details.is_empty() {
            self.downloads.next = 0.0;
            return false;
        }
        let wait = self.downloads.next - clock;
        if wait > 0.0 {
            tx.wake_after(Duration::from_secs_f64(wait));
            return false;
        }
        self.downloads.next = clock + 1.0;
        tx.wake_after(Duration::from_secs(1));
        let list = self.core.torrents();
        let changed = list != self.downloads.list;
        self.downloads.list = list;
        changed
    }

    fn torrent_command(&mut self, id: usize, command: Command) {
        let engine = match self.core.ready_engine() {
            Ok(engine) => engine,
            Err(err) => return self.toast_error(format!("{err:#}")),
        };
        let work = async move {
            match command {
                Command::Pause => engine.pause(id).await,
                Command::Resume => engine.resume(id).await,
                Command::Remove { delete_files } => engine.remove(id, delete_files).await,
            }
        };
        self.bg.run(work, |s, done, _| {
            if let Err(err) = done {
                s.toast_error(format!("{err:#}"));
            }
            // Show the change now rather than within the second.
            s.downloads.next = 0.0;
        });
    }
}

#[derive(Clone, Copy)]
enum Command {
    Pause,
    Resume,
    Remove { delete_files: bool },
}

pub fn page(s: &App, _: &mut rune::Cx<App>) -> El {
    let status = match &s.core.engine {
        EngineStatus::Starting => Some(busy("Starting the torrent engine…")),
        EngineStatus::Failed(err) => Some(error_text(format!(
            "The torrent engine isn't running: {err}"
        ))),
        EngineStatus::Off | EngineStatus::Ready(_) => None,
    };
    // The engine starts when needed, so "off" also means nothing downloading.
    let idle = matches!(s.core.engine, EngineStatus::Ready(_) | EngineStatus::Off);
    let mut content = col()
        .key("downloads")
        .w_full()
        .px(space::PAGE)
        .pt(BAR_H + 28.0)
        .pb(56.0)
        .gap(20.0)
        .child(page_title("Downloads"))
        .children(status);
    content = if s.downloads.list.is_empty() && idle {
        content.child(empty_state(
            &icons::DOWNLOAD,
            "Nothing downloading",
            "Torrent streams you play show up here while they download.",
        ))
    } else {
        content.child(
            col()
                .w_full()
                .gap(12.0)
                .children(s.downloads.list.iter().map(torrent_card)),
        )
    };
    scroller(s, Page::Downloads, content)
}

fn torrent_card(torrent: &TorrentInfo) -> El {
    let th = t();
    let state = match (&torrent.error, torrent.paused, torrent.finished) {
        (Some(_), ..) => tag("Error", Tone::Danger),
        (None, true, _) => tag("Paused", Tone::Neutral),
        (None, false, true) => tag("Finished", Tone::Success),
        (None, false, false) => tag(torrent.state.clone(), Tone::Info),
    };
    let stats = [
        format!(
            "{} of {}",
            fmt_bytes(torrent.downloaded_bytes),
            fmt_bytes(torrent.total_bytes)
        ),
        format!("↓ {}", fmt_speed(torrent.down_speed)),
        format!("↑ {}", fmt_speed(torrent.up_speed)),
        format!("{} peers", torrent.peers),
    ];
    let (id, paused, name) = (torrent.id, torrent.paused, torrent.name.clone());
    panel()
        .key(format!("torrent-{id}"))
        .padding(18.0)
        .gap(12.0)
        .child(
            row()
                .gap(12.0)
                .center_y()
                .child(
                    text(torrent.name.clone())
                        .font(ty::LABEL)
                        .color(th.text)
                        .truncate()
                        .grow(1.0)
                        .shrink(),
                )
                .child(state),
        )
        .child(progress(
            torrent.progress,
            6.0,
            th.raised2,
            if torrent.finished {
                th.success
            } else {
                th.accent
            },
        ))
        .child(
            row()
                .gap(16.0)
                .center_y()
                .justify_between()
                .child(
                    row()
                        .gap(18.0)
                        .children(stats.map(|s| text(s).font(ty::CAPTION).color(th.muted))),
                )
                .child(
                    row()
                        .gap(8.0)
                        .child(
                            Button::new(if paused { "Resume" } else { "Pause" })
                                .with_icon(if paused { &icons::PLAY } else { &icons::PAUSE })
                                .size(Size::Small)
                                .on_press(move |s: &mut App, _| {
                                    s.torrent_command(
                                        id,
                                        if paused {
                                            Command::Resume
                                        } else {
                                            Command::Pause
                                        },
                                    )
                                })
                                .build(),
                        )
                        .child(
                            Button::new("Remove")
                                .with_icon(&icons::TRASH)
                                .variant(Variant::Ghost)
                                .size(Size::Small)
                                .on_press(move |s: &mut App, _| {
                                    s.downloads.confirm = Some((id, name.clone()))
                                })
                                .build(),
                        ),
                ),
        )
        .children(torrent.error.clone().map(error_text))
}

/// "Remove this download?", while one waits on the answer.
pub fn confirm_dialog(s: &App) -> Option<El> {
    let (id, name) = s.downloads.confirm.clone()?;
    let th = t();
    let answer = move |delete_files: bool| {
        move |s: &mut App, _: &mut Tx| {
            s.downloads.confirm = None;
            s.torrent_command(id, Command::Remove { delete_files });
        }
    };
    let dialog = col().key("confirm-remove")
        .w(460.0)
        .padding(24.0)
        .gap(12.0)
        .radius(14.0)
        .fill(th.raised)
        .border(1.0, th.line)
        .box_shadow(float_shadow())
        .role("alertdialog")
        .label("Remove this download?")
        .modal(true, |s: &mut App| s.downloads.confirm = None)
        .transition(rune::Transition::scale(0.96).combined(rune::Transition::opacity()))
        .child(text("Remove this download?").font(ty::HEADING).color(th.text))
        .child(
            text(format!(
                "“{name}” will be removed from the list. You can keep the downloaded files or delete them too."
            ))
            .font(ty::BODY)
            .color(th.muted)
            .wrap(),
        )
        .child(
            row().gap(8.0)
                .pt(12.0)
                .justify_end()
                .child(Button::new("Cancel").variant(Variant::Ghost).on_press(|s: &mut App, _| s.downloads.confirm = None).build())
                .child(Button::new("Keep files").on_press(answer(false)).build())
                .child(Button::new("Delete files").variant(Variant::Danger).on_press(answer(true)).build()),
        );
    Some(
        el().key("confirm-layer")
            .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
            .center()
            .fill(Color::BLACK.alpha(0.55))
            .transition(rune::Transition::opacity())
            .child(dialog),
    )
}
