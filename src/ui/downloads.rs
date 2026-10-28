//! Downloads: the torrent engine's transfers, refreshed every second while
//! the page is on screen.

use std::time::{Duration, Instant};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Sizable, StyledExt, WindowExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::common::{
    busy, card, eid, empty_state, error_text, fmt_bytes, fmt_speed, page, page_title,
};
use crate::state::{AppState, EngineStatus};
use crate::torrent::TorrentInfo;

const REFRESH: Duration = Duration::from_secs(1);
/// The ticker stops once the page hasn't rendered for this long.
const IDLE_STOP: Duration = Duration::from_secs(3);

pub struct DownloadsView {
    state: Entity<AppState>,
    ticker: Option<Task<()>>,
    last_render: Instant,
}

impl DownloadsView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self {
            state,
            ticker: None,
            last_render: Instant::now(),
        }
    }

    /// Re-renders every second while the page keeps rendering; the engine's
    /// numbers change without the state entity noticing.
    fn keep_ticking(&mut self, cx: &mut Context<Self>) {
        self.last_render = Instant::now();
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(REFRESH).await;
                let keep_going = this.update(cx, |this, cx| {
                    if this.last_render.elapsed() > IDLE_STOP {
                        this.ticker = None;
                        false
                    } else {
                        cx.notify();
                        true
                    }
                });
                if !matches!(keep_going, Ok(true)) {
                    break;
                }
            }
        }));
    }

    fn run(&self, task: Task<anyhow::Result<()>>, window: &mut Window, cx: &mut Context<Self>) {
        cx.spawn_in(window, async move |this, cx| {
            if let Err(err) = task.await {
                this.update_in(cx, |_, window, cx| {
                    window.push_notification(Notification::error(format!("{err:#}")), cx);
                })
                .ok();
            }
        })
        .detach();
    }

    fn toggle_pause(&mut self, torrent: &TorrentInfo, window: &mut Window, cx: &mut Context<Self>) {
        let id = torrent.id;
        let paused = torrent.paused;
        let task = self.state.update(cx, |state, cx| {
            if paused {
                state.resume_torrent(id, cx)
            } else {
                state.pause_torrent(id, cx)
            }
        });
        self.run(task, window, cx);
    }

    fn confirm_remove(
        &mut self,
        torrent: &TorrentInfo,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = torrent.id;
        let name = torrent.name.clone();
        let this = cx.entity();
        window.open_alert_dialog(cx, move |dialog, _, _| {
            let remove = {
                let this = this.clone();
                move |delete_files: bool, window: &mut Window, cx: &mut App| {
                    this.update(cx, |this, cx| {
                        let task = this
                            .state
                            .update(cx, |state, cx| state.remove_torrent(id, delete_files, cx));
                        this.run(task, window, cx);
                    });
                    window.close_dialog(cx);
                }
            };
            let keep = remove.clone();
            dialog
                .title("Remove this download?")
                .description(format!("“{name}” will be removed from the list. You can keep the downloaded files or delete them too."))
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("remove-cancel")
                                .label("Cancel")
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("remove-keep")
                                .outline()
                                .label("Keep files")
                                .on_click(move |_, window, cx| keep(false, window, cx)),
                        )
                        .child(
                            Button::new("remove-delete")
                                .danger()
                                .label("Delete files")
                                .on_click(move |_, window, cx| remove(true, window, cx)),
                        ),
                )
        });
    }

    fn torrent_row(&self, torrent: TorrentInfo, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let state_label = match (&torrent.error, torrent.paused, torrent.finished) {
            (Some(_), ..) => Tag::danger().small().child("Error"),
            (None, true, _) => Tag::secondary().small().child("Paused"),
            (None, false, true) => Tag::success().small().child("Finished"),
            (None, false, false) => Tag::info().small().child(torrent.state.clone()),
        };
        let stats = [
            format!(
                "{} / {}",
                fmt_bytes(torrent.downloaded_bytes),
                fmt_bytes(torrent.total_bytes)
            ),
            format!("↓ {}", fmt_speed(torrent.down_speed)),
            format!("↑ {}", fmt_speed(torrent.up_speed)),
            format!("{} peers", torrent.peers),
        ];
        let pause = torrent.clone();
        let remove = torrent.clone();
        card(cx)
            .flex()
            .flex_col()
            .gap_3()
            .p_4()
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_medium()
                            .truncate()
                            .child(torrent.name.clone()),
                    )
                    .child(state_label),
            )
            .child(
                Progress::new(eid(format!("torrent-progress-{}", torrent.id)))
                    .value(torrent.progress.clamp(0.0, 1.0) * 100.)
                    .h(px(6.)),
            )
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .gap_4()
                    .child(
                        h_flex()
                            .gap_4()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .children(stats),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new(eid(format!("torrent-pause-{}", torrent.id)))
                                    .small()
                                    .outline()
                                    .icon(if torrent.paused {
                                        IconName::Play
                                    } else {
                                        IconName::Pause
                                    })
                                    .label(if torrent.paused { "Resume" } else { "Pause" })
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.toggle_pause(&pause, window, cx)
                                    })),
                            )
                            .child(
                                Button::new(eid(format!("torrent-remove-{}", torrent.id)))
                                    .small()
                                    .ghost()
                                    .icon(IconName::Trash)
                                    .label("Remove")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.confirm_remove(&remove, window, cx)
                                    })),
                            ),
                    ),
            )
            .when_some(torrent.error.clone(), |this, err| {
                this.child(error_text(err, cx))
            })
    }
}

impl Render for DownloadsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.keep_ticking(cx);
        let (torrents, status) = {
            let state = self.state.read(cx);
            let status = match &state.engine {
                EngineStatus::Starting => Some(busy("Starting the torrent engine…", cx)),
                EngineStatus::Failed(err) => Some(error_text(
                    format!("The torrent engine isn't running: {err}"),
                    cx,
                )),
                EngineStatus::Off | EngineStatus::Ready(_) => None,
            };
            (state.torrents(), status)
        };
        let ready = matches!(self.state.read(cx).engine, EngineStatus::Ready(_));
        let rows: Vec<Div> = torrents
            .into_iter()
            .map(|torrent| self.torrent_row(torrent, cx))
            .collect();

        page("downloads")
            .child(page_title("Downloads"))
            .when_some(status, |this, status| this.child(status))
            .map(|this| {
                if rows.is_empty() && ready {
                    this.child(empty_state(
                        IconName::Download,
                        "Nothing downloading",
                        "Torrent streams you play show up here while they download.",
                        cx,
                    ))
                } else {
                    this.child(v_flex().gap_3().children(rows))
                }
            })
    }
}
