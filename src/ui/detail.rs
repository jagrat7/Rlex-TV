//! Detail: one title, with its metadata, the sources of each video (copies
//! on this computer first, then addon streams), and for a series its
//! seasons and episodes.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::notification::Notification;
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt, WindowExt, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::DetailTarget;
use super::common::{
    Load, busy, card, eid, error_text, fmt_bytes, fmt_clock, fmt_date, heading, muted, page,
    skeleton_lines,
};
use crate::addons::{Meta, Video};
use crate::library::{LocalFile, LocalVideo};
use crate::state::{AppState, FoundStream, StreamResults};
use crate::watch::PlayItem;

const BACKDROP_HEIGHT: f32 = 320.;
const POSTER_WIDTH: f32 = 170.;
const POSTER_HEIGHT: f32 = 255.;

/// Where the title's metadata stands. Local-only titles never load any.
enum Info {
    Local,
    Loading,
    Ready(Box<Meta>),
    Failed(String),
}

pub struct DetailView {
    state: Entity<AppState>,
    target: DetailTarget,
    info: Info,
    season: Option<u32>,
    /// The selected episode's video id, for a series.
    episode: Option<String>,
    /// Streams for one video id, once asked for.
    streams: Option<(String, Load<StreamResults>)>,
    /// The file path or stream key whose player is starting up.
    starting: Option<String>,
    /// An episode to select as soon as the metadata arrives.
    preselect: Option<String>,
}

impl DetailView {
    pub fn new(state: Entity<AppState>, target: DetailTarget, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let mut this = Self {
            state,
            info: if target.is_local() {
                Info::Local
            } else {
                Info::Loading
            },
            target,
            season: None,
            episode: None,
            streams: None,
            starting: None,
            preselect: None,
        };
        if !this.target.is_local() {
            this.load_meta(cx);
        }
        this
    }

    fn load_meta(&mut self, cx: &mut Context<Self>) {
        let (kind, id) = (self.target.kind.clone(), self.target.id.clone());
        let task = self
            .state
            .update(cx, |state, cx| state.load_meta(&kind, &id, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                match result {
                    Ok(meta) => {
                        if meta.kind == "series" {
                            this.season = meta.seasons().into_iter().next();
                        } else {
                            this.load_streams(meta.id.clone(), cx);
                        }
                        this.target.name = meta.name.clone();
                        this.target.poster = meta.poster.clone();
                        this.info = Info::Ready(Box::new(meta));
                        if let Some(video_id) = this.preselect.take() {
                            this.select_episode(video_id, cx);
                        }
                    }
                    Err(err) => this.info = Info::Failed(format!("{err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn load_streams(&mut self, video_id: String, cx: &mut Context<Self>) {
        let kind = self.target.kind.clone();
        self.streams = Some((video_id.clone(), Load::Loading));
        let task = self
            .state
            .update(cx, |state, cx| state.load_streams(&kind, &video_id, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await;
            this.update(cx, |this, cx| {
                if this.streams.as_ref().is_some_and(|(id, _)| *id == video_id) {
                    this.streams = Some((video_id, Load::from_result(result)));
                    cx.notify();
                }
            })
            .ok();
        })
        .detach();
    }

    /// Selects an episode (and its season) once the metadata has loaded.
    #[cfg(debug_assertions)]
    pub fn preselect_episode(&mut self, video_id: String) {
        self.preselect = Some(video_id);
    }

    fn select_episode(&mut self, video_id: String, cx: &mut Context<Self>) {
        if let Info::Ready(meta) = &self.info {
            let season = meta
                .videos
                .iter()
                .find(|v| v.id == video_id)
                .and_then(|v| v.season);
            if season.is_some() {
                self.season = season;
            }
        }
        if self.episode.as_deref() == Some(&video_id) {
            self.episode = None;
            self.streams = None;
        } else {
            self.episode = Some(video_id.clone());
            self.load_streams(video_id, cx);
        }
        cx.notify();
    }

    /// The history entry a video is recorded under.
    fn play_item(&self, video: Option<&Video>) -> PlayItem {
        let (video_id, episode_label) = match video {
            Some(video) => (video.id.clone(), Some(video.label())),
            None => (self.target.id.clone(), None),
        };
        PlayItem {
            meta_id: self.target.id.clone(),
            kind: self.target.kind.clone(),
            name: self.target.name.clone(),
            poster: self.target.poster.clone(),
            video_id,
            episode_label,
        }
    }

    /// A history entry for a video of an unmatched title.
    fn local_play_item(&self, video: &LocalVideo) -> PlayItem {
        PlayItem {
            meta_id: self.target.id.clone(),
            kind: self.target.kind.clone(),
            name: self.target.name.clone(),
            poster: None,
            video_id: video.video_id.clone(),
            episode_label: video.versions.first().and_then(LocalFile::episode_label),
        }
    }

    fn play_local(
        &mut self,
        file: LocalFile,
        item: PlayItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = file.path.to_string_lossy().into_owned();
        let task = self
            .state
            .update(cx, |state, cx| state.play_local(&file, item, cx));
        self.wait_for_player(key, task, window, cx);
    }

    fn play_stream(
        &mut self,
        found: FoundStream,
        item: PlayItem,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let key = found.stream.dedupe_key();
        let task = self
            .state
            .update(cx, |state, cx| state.play_stream(&found, item, cx));
        self.wait_for_player(key, task, window, cx);
    }

    /// Marks `key` as starting until the player is up, then reports failure.
    fn wait_for_player(
        &mut self,
        key: String,
        task: Task<anyhow::Result<()>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.starting.is_some() {
            return;
        }
        self.starting = Some(key);
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.starting = None;
                if let Err(err) = result {
                    window.push_notification(
                        Notification::error(format!("Couldn't play: {err:#}")),
                        cx,
                    );
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn set_watched(&mut self, item: PlayItem, watched: bool, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.set_watched(&item, watched, cx));
    }

    // Rendering

    fn header(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let meta = match &self.info {
            Info::Ready(meta) => Some(meta.as_ref()),
            _ => None,
        };
        let background = meta.and_then(|m| m.background.clone());
        let poster = meta
            .and_then(|m| m.poster.clone())
            .or_else(|| self.target.poster.clone());
        let name: SharedString = meta
            .map(|m| m.name.clone())
            .unwrap_or_else(|| self.target.name.clone())
            .into();

        let mut facts: Vec<String> = Vec::new();
        if let Some(meta) = meta {
            facts.extend(meta.year_label().map(str::to_string));
            facts.extend(meta.imdb_rating.as_ref().map(|r| format!("★ {r}")));
            facts.extend(meta.runtime.clone());
        }
        let genres: Vec<String> = meta.map(|m| m.genres().to_vec()).unwrap_or_default();

        let fallback_bg = theme.muted;
        let fallback_fg = theme.muted_foreground;
        let fallback_name = name.clone();
        let fallback = move || {
            div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .p_3()
                .bg(fallback_bg)
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_center()
                        .text_color(fallback_fg)
                        .line_clamp(5)
                        .child(fallback_name.clone()),
                )
                .into_any_element()
        };
        let poster_image: AnyElement = match poster {
            Some(url) => img(url)
                .size_full()
                .object_fit(ObjectFit::Cover)
                .with_fallback(fallback)
                .into_any_element(),
            None => fallback(),
        };

        div()
            .relative()
            .w_full()
            .h(px(BACKDROP_HEIGHT))
            .overflow_hidden()
            .bg(theme.secondary)
            .when_some(background, |this, url| {
                this.child(
                    img(url)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
            })
            .child(div().absolute().inset_0().bg(linear_gradient(
                180.,
                linear_color_stop(theme.background.alpha(0.35), 0.),
                linear_color_stop(theme.background.alpha(0.96), 1.),
            )))
            .child(
                h_flex()
                    .absolute()
                    .bottom_0()
                    .left_0()
                    .right_0()
                    .px_6()
                    .pb_5()
                    .gap_6()
                    .items_end()
                    .child(
                        div()
                            .w(px(POSTER_WIDTH))
                            .h(px(POSTER_HEIGHT))
                            .flex_none()
                            .rounded(theme.radius_lg)
                            .overflow_hidden()
                            .bg(theme.muted)
                            .child(poster_image),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(div().text_3xl().font_bold().truncate().child(name))
                            .when(!facts.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.muted_foreground)
                                        .child(facts.join("  ·  ")),
                                )
                            })
                            .when(!genres.is_empty(), |this| {
                                this.child(
                                    h_flex().gap_2().flex_wrap().children(
                                        genres
                                            .into_iter()
                                            .map(|genre| Tag::secondary().small().child(genre)),
                                    ),
                                )
                            }),
                    ),
            )
    }

    fn about(&self, meta: &Meta, cx: &mut Context<Self>) -> Div {
        let people = |label: &str, names: &[String]| {
            (!names.is_empty()).then(|| {
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .flex_none()
                            .child(format!("{label}:")),
                    )
                    .child(div().child(names.join(", ")))
            })
        };
        v_flex()
            .gap_3()
            .max_w(px(900.))
            .when_some(meta.description.clone(), |this, text| {
                this.child(div().text_sm().line_height(rems(1.5)).child(text))
            })
            .children(people("Director", &meta.director))
            .children(people("Cast", &meta.cast))
    }

    fn body(&self, cx: &mut Context<Self>) -> AnyElement {
        match &self.info {
            Info::Loading => v_flex()
                .gap_6()
                .child(skeleton_lines(3, cx))
                .child(Skeleton::new().w_1_3().h_6())
                .child(skeleton_lines(2, cx))
                .into_any_element(),
            Info::Failed(err) => {
                error_text(format!("Couldn't load details: {err}"), cx).into_any_element()
            }
            Info::Local => self.local_body(cx).into_any_element(),
            Info::Ready(meta) => {
                let meta = meta.clone();
                let main = if meta.kind == "series" {
                    self.series_body(&meta, cx)
                } else {
                    self.movie_body(&meta, cx)
                };
                v_flex()
                    .gap_6()
                    .child(self.about(&meta, cx))
                    .child(main)
                    .into_any_element()
            }
        }
    }

    /// A title that matched nothing online: its own files, and nothing else.
    fn local_body(&self, cx: &mut Context<Self>) -> Div {
        let videos = self
            .state
            .read(cx)
            .library_title(&self.target.id)
            .map(|title| title.videos())
            .unwrap_or_default();
        let note = muted(
            "This title didn't match anything online, so there are no details or streams. Its files are below.",
            cx,
        );
        if videos.is_empty() {
            return v_flex()
                .gap_6()
                .child(note)
                .child(muted("No files found for this title.", cx));
        }
        if self.target.kind != "series" {
            let video = &videos[0];
            let item = self.local_play_item(video);
            return v_flex()
                .gap_6()
                .child(note)
                .child(self.actions("movie", &video.versions, item.clone(), cx))
                .child(self.sources_section(&video.versions, None, item, cx));
        }

        let mut blocks: Vec<Div> = Vec::new();
        let mut current_season = None;
        for (index, video) in videos.iter().enumerate() {
            if blocks.is_empty() || video.season != current_season {
                current_season = video.season;
                blocks.push(heading(match video.season {
                    Some(0) => "Specials".to_string(),
                    Some(s) => format!("Season {s}"),
                    None => "Episodes".to_string(),
                }));
            }
            let item = self.local_play_item(video);
            let label = item
                .episode_label
                .clone()
                .unwrap_or_else(|| video.versions[0].file_name());
            blocks.push(
                v_flex()
                    .gap_3()
                    .child(div().text_sm().font_medium().child(label))
                    .child(self.actions(
                        format!("local-{index}"),
                        &video.versions,
                        item.clone(),
                        cx,
                    ))
                    .child(self.source_rows(&video.versions, item, cx)),
            );
        }
        v_flex().gap_6().child(note).children(blocks)
    }

    fn movie_body(&self, meta: &Meta, cx: &mut Context<Self>) -> Div {
        let item = self.play_item(None);
        let files = self.state.read(cx).local_files(&meta.id, None);
        v_flex()
            .gap_6()
            .child(self.actions("movie", &files, item.clone(), cx))
            .child(self.sources_section(&files, Some(&meta.id), item, cx))
    }

    fn series_body(&self, meta: &Meta, cx: &mut Context<Self>) -> Div {
        let seasons = meta.seasons();
        let season = self.season.or_else(|| seasons.first().copied());
        let episodes: Vec<Video> = season
            .map(|s| meta.episodes(s).into_iter().cloned().collect())
            .unwrap_or_default();

        let selector = h_flex()
            .gap_2()
            .flex_wrap()
            .children(seasons.iter().map(|&s| {
                let label = if s == 0 {
                    "Specials".to_string()
                } else {
                    format!("Season {s}")
                };
                Button::new(eid(format!("season-{s}")))
                    .small()
                    .outline()
                    .selected(season == Some(s))
                    .label(label)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.season = Some(s);
                        this.episode = None;
                        this.streams = None;
                        cx.notify();
                    }))
            }));

        let list = v_flex().gap_1().children(
            episodes
                .into_iter()
                .enumerate()
                .map(|(i, video)| self.episode_row(i, video, cx)),
        );

        v_flex()
            .gap_4()
            .child(heading("Episodes"))
            .child(selector)
            .map(|this| {
                if seasons.is_empty() {
                    this.child(muted("No episodes listed yet.", cx))
                } else {
                    this.child(list)
                }
            })
    }

    fn episode_row(&self, index: usize, video: Video, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let selected = self.episode.as_deref() == Some(video.id.as_str());
        let entry = self.state.read(cx).watch_entry(&video.id).cloned();
        let watched = entry.as_ref().is_some_and(|e| e.watched);
        let progress = entry
            .as_ref()
            .map(|e| e.progress())
            .filter(|p| *p > 0.0 && !watched);
        let files = self
            .state
            .read(cx)
            .local_files(&self.target.id, Some(&video.id));
        let video_id = video.id.clone();

        let on_disk = match files.len() {
            0 => None,
            1 => Some("On disk".to_string()),
            n => Some(format!("{n} copies")),
        };
        let status: AnyElement = if watched {
            Icon::new(IconName::CircleCheck)
                .size_4()
                .text_color(theme.success)
                .into_any_element()
        } else if let Some(progress) = progress {
            div()
                .w(px(72.))
                .h(px(4.))
                .rounded_full()
                .bg(theme.muted)
                .child(
                    div()
                        .h_full()
                        .rounded_full()
                        .w(relative(progress))
                        .bg(theme.primary),
                )
                .into_any_element()
        } else {
            div().into_any_element()
        };

        let row = h_flex()
            .id(eid(format!("episode-{index}")))
            .w_full()
            .gap_3()
            .px_3()
            .py_2()
            .items_center()
            .rounded(theme.radius)
            .cursor_pointer()
            .bg(if selected {
                theme.list_active
            } else {
                theme.transparent
            })
            .hover(|this| this.bg(theme.list_hover))
            .on_click(cx.listener(move |this, _, _, cx| this.select_episode(video_id.clone(), cx)))
            .when_some(video.thumbnail.clone(), |this, url| {
                this.child(
                    div()
                        .w(px(96.))
                        .h(px(54.))
                        .flex_none()
                        .rounded(theme.radius)
                        .overflow_hidden()
                        .bg(theme.muted)
                        .child(img(url).size_full().object_fit(ObjectFit::Cover)),
                )
            })
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .truncate()
                            .child(video.label()),
                    )
                    .when_some(video.released.as_deref(), |this, released| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(fmt_date(released)),
                        )
                    }),
            )
            .when_some(on_disk, |this, label| {
                this.child(Tag::secondary().small().child(label))
            })
            .child(div().flex_none().child(status));

        if !selected {
            return div().w_full().child(row);
        }
        let (accent, muted_fg) = (theme.primary, theme.muted_foreground);
        let item = self.play_item(Some(&video));
        v_flex().gap_1().child(row).child(
            v_flex()
                .gap_5()
                .ml_3()
                .pl_4()
                .py_3()
                .border_l_2()
                .border_color(accent)
                .when_some(
                    video.overview().filter(|o| !o.is_empty()),
                    |this, overview| {
                        this.child(
                            div()
                                .text_sm()
                                .max_w(px(900.))
                                .text_color(muted_fg)
                                .child(overview.to_string()),
                        )
                    },
                )
                .child(self.actions("episode", &files, item.clone(), cx))
                .child(self.sources_section(&files, Some(&video.id), item, cx)),
        )
    }

    /// The primary Play (the best local copy, resuming where the history
    /// says) and the watched toggle. Without a local copy, Play is left to
    /// the source list and the resume position is shown as a tag.
    fn actions(
        &self,
        id: impl Into<String>,
        files: &[LocalFile],
        item: PlayItem,
        cx: &mut Context<Self>,
    ) -> Div {
        let id = id.into();
        let entry = self.state.read(cx).watch_entry(&item.video_id).cloned();
        let watched = entry.as_ref().is_some_and(|e| e.watched);
        let resume = entry.as_ref().and_then(|e| e.resume_position());
        let best = files.first().cloned();
        let starting = best
            .as_ref()
            .is_some_and(|f| self.starting.as_deref() == Some(&*f.path.to_string_lossy()));
        let toggle_item = item.clone();
        h_flex()
            .gap_3()
            .items_center()
            .when_some(best, |this, file| {
                let label = match (starting, resume) {
                    (true, _) => "Starting…".to_string(),
                    (false, Some(position)) => format!("Resume from {}", fmt_clock(position)),
                    (false, None) => "Play".to_string(),
                };
                this.child(
                    Button::new(eid(format!("{id}-play")))
                        .primary()
                        .icon(IconName::Play)
                        .label(label)
                        .loading(starting)
                        .disabled(self.starting.is_some() && !starting)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.play_local(file.clone(), item.clone(), window, cx)
                        })),
                )
            })
            .child(
                Button::new(eid(format!("{id}-watched")))
                    .outline()
                    .icon(if watched {
                        IconName::CircleCheck
                    } else {
                        IconName::Circle
                    })
                    .label(if watched { "Watched" } else { "Mark watched" })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_watched(toggle_item.clone(), !watched, cx)
                    })),
            )
            .when(files.is_empty(), |this| {
                this.when_some(resume, |this, position| {
                    this.child(
                        Tag::info()
                            .small()
                            .child(format!("Resume from {}", fmt_clock(position))),
                    )
                })
            })
    }

    /// Everything that can play one video: copies on this computer, then
    /// addon streams when `stream_video_id` is given.
    fn sources_section(
        &self,
        files: &[LocalFile],
        stream_video_id: Option<&str>,
        item: PlayItem,
        cx: &mut Context<Self>,
    ) -> Div {
        let streams: Option<AnyElement> = stream_video_id.map(|video_id| {
            let results = match &self.streams {
                Some((id, results)) if id == video_id => Some(results),
                _ => None,
            };
            match results {
                None | Some(Load::Loading) => {
                    busy("Looking for streams… torrents can take a few seconds.", cx)
                        .into_any_element()
                }
                Some(Load::Failed(err)) => {
                    error_text(format!("Couldn't look for streams: {err}"), cx).into_any_element()
                }
                Some(Load::Ready(results)) => self
                    .stream_list(results, files.is_empty(), item.clone(), cx)
                    .into_any_element(),
            }
        });
        v_flex().gap_3().child(heading("Sources")).child(
            v_flex()
                .gap_2()
                .child(self.source_rows(files, item, cx))
                .children(streams),
        )
    }

    /// One row per local copy, best first.
    fn source_rows(&self, files: &[LocalFile], item: PlayItem, cx: &mut Context<Self>) -> Div {
        let several = files.len() > 1;
        v_flex()
            .gap_2()
            .children(files.iter().enumerate().map(|(i, file)| {
                self.local_source_row(i, file.clone(), several && i == 0, item.clone(), cx)
            }))
    }

    fn local_source_row(
        &self,
        index: usize,
        file: LocalFile,
        best: bool,
        item: PlayItem,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let key = file.path.to_string_lossy().into_owned();
        let quality = match file.quality.label() {
            label if label.is_empty() => "Unknown quality".to_string(),
            label => label,
        };
        let folder = file
            .path
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let detail = v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(div().text_sm().font_medium().child("This computer"))
                    .child(Tag::secondary().small().child(quality))
                    .when(best, |this| {
                        this.child(Tag::success().small().child("Best"))
                    }),
            )
            .child(div().text_xs().truncate().child(format!(
                "{}  ·  {}",
                file.file_name(),
                fmt_bytes(file.size)
            )))
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(folder),
            );
        self.source_row(
            eid(format!("play-file-{index}-{key}")),
            key,
            IconName::HardDrive,
            detail,
            cx.listener(move |this, _, window, cx| {
                this.play_local(file.clone(), item.clone(), window, cx)
            }),
            cx,
        )
    }

    fn stream_list(
        &self,
        results: &StreamResults,
        no_local: bool,
        item: PlayItem,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        if results.asked == 0 {
            return muted("Add a source in Settings to find streams.", cx);
        }
        let playable: Vec<&FoundStream> = results
            .streams
            .iter()
            .filter(|f| f.stream.is_playable())
            .collect();
        let failures = v_flex()
            .gap_1()
            .children(results.failures.iter().map(|(source, err)| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(format!("{source} didn't answer: {err}"))
            }));
        v_flex()
            .gap_2()
            .map(|this| {
                if playable.is_empty() {
                    this.when(no_local, |this| {
                        this.child(muted("No streams found for this video.", cx))
                    })
                } else {
                    this.children(
                        playable
                            .into_iter()
                            .enumerate()
                            .map(|(i, found)| self.stream_row(i, found.clone(), item.clone(), cx)),
                    )
                }
            })
            .when(!results.failures.is_empty(), |this| this.child(failures))
    }

    fn stream_row(
        &self,
        index: usize,
        found: FoundStream,
        item: PlayItem,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let key = found.stream.dedupe_key();
        let lines: Vec<String> = found
            .stream
            .text()
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect();
        let kind_tag = if found.stream.is_torrent() {
            Tag::warning().small().child("Torrent")
        } else {
            Tag::success().small().child("Direct")
        };
        let detail = v_flex()
            .flex_1()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .text_sm()
                            .font_medium()
                            .truncate()
                            .child(found.source.clone()),
                    )
                    .when_some(found.stream.quality(), |this, quality| {
                        this.child(Tag::secondary().small().child(quality))
                    })
                    .child(kind_tag),
            )
            .children(lines.into_iter().map(|line| {
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(line)
            }));
        self.source_row(
            eid(format!("play-stream-{index}")),
            key,
            IconName::Globe,
            detail,
            cx.listener(move |this, _, window, cx| {
                this.play_stream(found.clone(), item.clone(), window, cx)
            }),
            cx,
        )
    }

    /// The shared shape of a source row: an icon, the details, and Play.
    fn source_row(
        &self,
        id: ElementId,
        key: String,
        icon: IconName,
        detail: Div,
        on_play: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let starting = self.starting.as_deref() == Some(key.as_str());
        let busy_elsewhere = self.starting.is_some() && !starting;
        card(cx)
            .flex()
            .flex_row()
            .items_center()
            .gap_4()
            .px_4()
            .py_3()
            .child(Icon::new(icon).size_4().text_color(theme.muted_foreground))
            .child(detail)
            .child(
                Button::new(id)
                    .primary()
                    .small()
                    .icon(IconName::Play)
                    .label(if starting { "Starting…" } else { "Play" })
                    .loading(starting)
                    .disabled(busy_elsewhere)
                    .on_click(on_play),
            )
    }
}

impl Render for DetailView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.header(cx);
        let body = self.body(cx);
        page("detail")
            .p_0()
            .gap_0()
            .child(header)
            .child(v_flex().gap_6().p_6().child(body))
    }
}
