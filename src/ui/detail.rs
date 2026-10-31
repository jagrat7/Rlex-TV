//! Detail: one title under its artwork, with one Play that picks the best
//! source and resumes (for a series, the next-up episode). Episodes play
//! when clicked. The full source list (copies on this computer first, then
//! addon streams) folds out for when you want to choose.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::skeleton::Skeleton;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::common::{
    Load, TOP_BAR_HEIGHT, await_player, busy, card, eid, error_text, fmt_bytes, fmt_clock,
    fmt_date, fmt_minutes, fmt_runtime, fmt_time_left, heading, hero_button, muted, page,
    runtime_minutes, short_episode, skeleton_lines, still_starting,
};
use super::poster::{PosterCard, poster_row, skeleton_row};
use super::quickplay::{self, next_up, play_label, play_title, play_video};
use super::ratings::RatingBadges;
use super::{DetailTarget, NavEvent};
use crate::addons::{Meta, Video};
use crate::library::{LocalFile, LocalVideo};
use crate::state::{AppState, FoundStream, StreamResults};
use crate::watch::PlayItem;

const POSTER_WIDTH: f32 = 170.;
const POSTER_HEIGHT: f32 = 255.;

/// Where the title's metadata stands. Local-only titles never load any.
enum Info {
    Local,
    Loading,
    Ready(Box<Meta>),
    Failed(String),
}

/// What the hero's Play button starts.
enum Primary {
    Video(PlayItem),
    /// An unmatched title: its next file, found at click time.
    LocalTitle,
    /// Nothing on this computer and no sources: offer to add one.
    NoSource,
}

/// Descriptions longer than this are cut in the hero, so they're shown in
/// full under it too.
const LONG_DESCRIPTION: usize = 320;

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
    /// Whether a movie's source list is folded out.
    show_sources: bool,
    /// Popular titles sharing the first genre, once asked for.
    similar: Option<Load<Vec<Meta>>>,
}

impl EventEmitter<NavEvent> for DetailView {}

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
            show_sources: false,
            similar: None,
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
                            // Open on the season you're watching.
                            let watching =
                                next_up(&meta, &this.state.read(cx).watch).and_then(|v| v.season);
                            this.season = watching.or_else(|| meta.seasons().into_iter().next());
                        } else {
                            this.load_streams(meta.id.clone(), cx);
                        }
                        let id = meta.id.clone();
                        this.state
                            .update(cx, |state, cx| state.load_ratings(&id, cx))
                            .detach();
                        this.load_similar(&meta, cx);
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

    fn load_similar(&mut self, meta: &Meta, cx: &mut Context<Self>) {
        let Some(genre) = meta.genres().first().cloned() else {
            return;
        };
        self.similar = Some(Load::Loading);
        let (kind, own_id) = (meta.kind.clone(), meta.id.clone());
        let task = self
            .state
            .update(cx, |state, cx| state.load_genre(&kind, &genre, cx));
        cx.spawn(async move |this, cx| {
            let result = task.await.map(|items| {
                items
                    .into_iter()
                    .filter(|m| m.id != own_id)
                    .take(20)
                    .collect::<Vec<_>>()
            });
            this.update(cx, |this, cx| {
                this.similar = Some(Load::from_result(result));
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
        quickplay::play_item(&self.target, video)
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
        if self.starting.is_some() {
            still_starting(window, cx);
            return;
        }
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
        if self.starting.is_some() {
            still_starting(window, cx);
            return;
        }
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
        self.starting = Some(key);
        cx.notify();
        await_player(task, window, cx, |this: &mut Self| this.starting = None);
    }

    fn set_watched(&mut self, item: PlayItem, watched: bool, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.set_watched(&item, watched, cx));
    }

    // Rendering

    /// What the hero's Play starts, and what its button says: the movie,
    /// the series' next-up episode, or an unmatched title's next file.
    /// `None` until there's something to play.
    fn primary(&self, cx: &App) -> Option<(Primary, String)> {
        let state = self.state.read(cx);
        let nothing_to_play = !self.target.is_local()
            && state.sources().is_empty()
            && state.local_files(&self.target.id, None).is_empty();
        if nothing_to_play && matches!(self.info, Info::Ready(_)) {
            return Some((Primary::NoSource, "Add a source to play".to_string()));
        }
        match &self.info {
            Info::Ready(meta) if meta.kind == "series" => {
                let video = next_up(meta, &state.watch)?;
                let short = short_episode(&video.label());
                let text = play_label(state, &video.id, Some(&short));
                Some((Primary::Video(self.play_item(Some(video))), text))
            }
            Info::Ready(_) => {
                let text = play_label(state, &self.target.id, None);
                Some((Primary::Video(self.play_item(None)), text))
            }
            Info::Local => {
                let title = state.library_title(&self.target.id)?;
                (!title.files.is_empty()).then(|| (Primary::LocalTitle, "Play".to_string()))
            }
            Info::Loading | Info::Failed(_) => None,
        }
    }

    /// The streams already found for `video_id`, so Play doesn't ask again.
    fn known_streams(&self, video_id: &str) -> Option<StreamResults> {
        match &self.streams {
            Some((id, Load::Ready(results))) if id == video_id => Some(results.clone()),
            _ => None,
        }
    }

    fn play_primary(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((primary, _)) = self.primary(cx) else {
            return;
        };
        match primary {
            Primary::Video(item) => self.quick_play(item, window, cx),
            Primary::NoSource => cx.emit(NavEvent::OpenSettings),
            Primary::LocalTitle => {
                if self.starting.is_some() {
                    still_starting(window, cx);
                    return;
                }
                self.starting = Some(self.target.id.clone());
                cx.notify();
                let task = play_title(&self.state, self.target.clone(), cx);
                await_player(task, window, cx, |this: &mut Self| this.starting = None);
            }
        }
    }

    /// Plays one video with the best source there is, no questions asked.
    fn quick_play(&mut self, item: PlayItem, window: &mut Window, cx: &mut Context<Self>) {
        if self.starting.is_some() {
            still_starting(window, cx);
            return;
        }
        let known = self.known_streams(&item.video_id);
        self.starting = Some(item.video_id.clone());
        cx.notify();
        let task = play_video(&self.state, item, known, cx);
        await_player(task, window, cx, |this: &mut Self| this.starting = None);
    }

    fn header(&self, window: &Window, cx: &mut Context<Self>) -> Div {
        let actions = self.hero_actions(cx);
        let theme = cx.theme();
        let bg = theme.background;
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
        // Titles with metadata keep the backdrop's height from the start, so
        // nothing jumps when the artwork arrives.
        let height = if !self.target.is_local() {
            (window.viewport_size().height * 0.62).clamp(px(400.), px(580.))
        } else {
            px(TOP_BAR_HEIGHT + POSTER_HEIGHT + 48.)
        };

        let mut facts: Vec<String> = Vec::new();
        if let Some(meta) = meta {
            facts.extend(meta.year_label().map(str::to_string));
            // A series' runtime is per episode, so it's shown on the episodes.
            if meta.kind != "series" {
                facts.extend(meta.runtime.as_deref().map(fmt_runtime));
            }
            if meta.kind == "series" {
                facts.push(match meta.seasons().iter().filter(|&&s| s > 0).count() {
                    0 => "Series".to_string(),
                    1 => "1 season".to_string(),
                    n => format!("{n} seasons"),
                });
            }
        }
        let genres: Vec<String> = meta
            .map(|m| m.genres().iter().take(4).cloned().collect())
            .unwrap_or_default();
        let ratings = (!self.target.is_local())
            .then(|| {
                RatingBadges::from_ratings(
                    self.state.read(cx).ratings(&self.target.id).as_ref(),
                    meta.and_then(|m| m.imdb_rating.as_deref()),
                )
            })
            .filter(|badges| !badges.is_empty());

        let title_text = {
            let name = name.clone();
            move || {
                div()
                    .text_size(px(40.))
                    .line_height(px(46.))
                    .font_bold()
                    .line_clamp(2)
                    .text_ellipsis()
                    .child(name.clone())
                    .into_any_element()
            }
        };
        let title: AnyElement = match meta.and_then(|m| m.logo.clone()) {
            Some(logo) if background.is_some() => div()
                .flex()
                .items_end()
                .child(
                    img(logo)
                        .h(px(100.))
                        .max_w(px(380.))
                        .object_fit(ObjectFit::Contain)
                        .with_fallback(title_text.clone()),
                )
                .into_any_element(),
            _ => title_text(),
        };

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
                        .text_ellipsis()
                        .child(fallback_name.clone()),
                )
                .into_any_element()
        };
        // The poster stands in for missing backdrop art.
        let poster_tile = background.is_none().then(|| {
            div()
                .w(px(POSTER_WIDTH))
                .h(px(POSTER_HEIGHT))
                .flex_none()
                .rounded(theme.radius_lg)
                .overflow_hidden()
                .bg(theme.muted)
                .child(match poster {
                    Some(url) => img(url)
                        .size_full()
                        .object_fit(ObjectFit::Cover)
                        .with_fallback(fallback)
                        .into_any_element(),
                    None => fallback(),
                })
        });

        let text = v_flex()
            .flex_1()
            .min_w_0()
            .max_w(px(560.))
            .gap_3()
            .child(title)
            .child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .flex_wrap()
                    .children(ratings)
                    .when(!facts.is_empty(), |this| {
                        this.child(
                            div()
                                .text_sm()
                                .text_color(theme.foreground.alpha(0.85))
                                .child(facts.join("  ·  ")),
                        )
                    }),
            )
            .when(!genres.is_empty(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(genres.join("  •  ")),
                )
            })
            .when_some(meta.and_then(|m| m.description.clone()), |this, text| {
                this.child(
                    div()
                        .text_sm()
                        .line_height(rems(1.45))
                        .text_color(theme.foreground.alpha(0.9))
                        .line_clamp(4)
                        .text_ellipsis()
                        .child(text),
                )
            })
            .child(actions);

        div()
            .relative()
            .w_full()
            .h(height)
            .overflow_hidden()
            .bg(theme.secondary.opacity(0.5))
            .when_some(background, |this, url| {
                this.child(
                    img(url)
                        .absolute()
                        .inset_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
                .child(div().absolute().inset_0().bg(linear_gradient(
                    90.,
                    linear_color_stop(bg.alpha(0.92), 0.),
                    linear_color_stop(bg.alpha(0.0), 0.7),
                )))
            })
            .child(div().absolute().inset_0().bg(linear_gradient(
                180.,
                linear_color_stop(bg.alpha(0.0), 0.5),
                linear_color_stop(bg, 1.),
            )))
            .child(
                h_flex()
                    .absolute()
                    .left_6()
                    .right_6()
                    .bottom_6()
                    .gap_6()
                    .items_end()
                    .children(poster_tile)
                    .child(text),
            )
    }

    /// Play (the best source, resuming), then for a movie the watched toggle
    /// and the source list.
    fn hero_actions(&self, cx: &mut Context<Self>) -> Div {
        let primary = self.primary(cx);
        let busy = match (&primary, &self.starting) {
            (Some((Primary::Video(item), _)), Some(key)) => *key == item.video_id,
            (Some((Primary::LocalTitle, _)), Some(key)) => *key == self.target.id,
            _ => false,
        };
        let no_source = matches!(primary, Some((Primary::NoSource, _)));
        let movie = self.target.kind != "series";
        let mut row = h_flex().pt_2().gap_3().items_center();
        match primary {
            Some((_, label)) => {
                let label = if busy {
                    "Starting…".to_string()
                } else {
                    label
                };
                row = row.child(hero_button(
                    "detail-play",
                    if no_source {
                        IconName::Plus
                    } else {
                        IconName::Play
                    },
                    label,
                    !no_source,
                    cx.listener(|this, _, window, cx| this.play_primary(window, cx)),
                ));
            }
            None if matches!(self.info, Info::Loading) => {
                row = row.child(Skeleton::new().w(px(150.)).h(px(42.)).rounded(px(6.)));
            }
            None => {}
        }
        if movie && !matches!(self.info, Info::Loading | Info::Failed(_)) {
            let item = match &self.info {
                Info::Local => self
                    .state
                    .read(cx)
                    .library_title(&self.target.id)
                    .and_then(|t| t.videos().into_iter().next())
                    .map(|v| self.local_play_item(&v)),
                _ => Some(self.play_item(None)),
            };
            if let Some(item) = item {
                let watched = self
                    .state
                    .read(cx)
                    .watch_entry(&item.video_id)
                    .is_some_and(|e| e.watched);
                row = row.child(hero_button(
                    "detail-watched",
                    if watched {
                        IconName::CircleCheck
                    } else {
                        IconName::Circle
                    },
                    if watched { "Watched" } else { "Mark watched" },
                    false,
                    cx.listener(move |this, _, _, cx| this.set_watched(item.clone(), !watched, cx)),
                ));
            }
            if !self.target.is_local() && !no_source {
                let count = match &self.streams {
                    Some((_, Load::Ready(results))) => results
                        .streams
                        .iter()
                        .filter(|f| f.stream.is_playable())
                        .count(),
                    _ => 0,
                };
                let local = self.state.read(cx).local_files(&self.target.id, None).len();
                let label = match count + local {
                    0 => "Sources".to_string(),
                    n => format!("Sources ({n})"),
                };
                row = row.child(hero_button(
                    "detail-sources",
                    if self.show_sources {
                        IconName::ChevronUp
                    } else {
                        IconName::ChevronDown
                    },
                    label,
                    false,
                    cx.listener(|this, _, _, cx| {
                        this.show_sources = !this.show_sources;
                        cx.notify();
                    }),
                ));
            }
        }
        row
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
            .when_some(
                meta.description
                    .clone()
                    .filter(|text| text.len() > LONG_DESCRIPTION),
                |this, text| this.child(div().text_sm().line_height(rems(1.5)).child(text)),
            )
            .children(people("Director", &meta.director))
            .children(people("Cast", &meta.cast))
    }

    /// Popular titles in the same genre, Netflix's "More like this".
    fn similar_row(&self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let body: AnyElement = match self.similar.as_ref()? {
            Load::Loading => skeleton_row(cx).into_any_element(),
            Load::Failed(_) => return None,
            Load::Ready(items) if items.is_empty() => return None,
            Load::Ready(items) => {
                let cards: Vec<PosterCard> = items
                    .iter()
                    .enumerate()
                    .map(|(i, meta)| {
                        let target = DetailTarget::from_meta(meta);
                        let play = target.clone();
                        PosterCard::new(eid(format!("similar-{i}")), meta.name.clone())
                            .poster(meta.poster.clone())
                            .subtitle(meta.year_label().map(str::to_string))
                            .rating(meta.imdb_rating.clone())
                            .busy(self.starting.as_deref() == Some(meta.id.as_str()))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(NavEvent::OpenDetail(target.clone()))
                            }))
                            .on_play(cx.listener(move |this, _, window, cx| {
                                if this.starting.is_some() {
                                    still_starting(window, cx);
                                    return;
                                }
                                this.starting = Some(play.id.clone());
                                cx.notify();
                                let task = play_title(&this.state, play.clone(), cx);
                                await_player(task, window, cx, |this: &mut Self| {
                                    this.starting = None
                                });
                            }))
                    })
                    .collect();
                poster_row("similar-row", cards, window, cx).into_any_element()
            }
        };
        Some(
            v_flex()
                .gap_3()
                .child(heading("More like this"))
                .child(body),
        )
    }

    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
                let similar = self.similar_row(window, cx);
                v_flex()
                    .gap_8()
                    .child(main)
                    .children(similar)
                    .child(self.about(&meta, cx))
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
            return v_flex().gap_6().child(note).child(self.sources_section(
                &video.versions,
                None,
                item,
                cx,
            ));
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
        v_flex().gap_6().when(self.show_sources, |this| {
            this.child(self.sources_section(&files, Some(&meta.id), item, cx))
        })
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

        let next = next_up(meta, &self.state.read(cx).watch).map(|v| v.id.clone());
        // Cinemeta gives one typical episode length per series.
        let runtime = meta.runtime.as_deref().and_then(runtime_minutes);
        let list = v_flex().children(episodes.into_iter().enumerate().map(|(i, video)| {
            let is_next = next.as_deref() == Some(video.id.as_str());
            self.episode_row(i, video, is_next, runtime, cx)
        }));

        v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_6()
                    .items_center()
                    .flex_wrap()
                    .child(heading("Episodes"))
                    .child(selector),
            )
            .map(|this| {
                if seasons.is_empty() {
                    this.child(muted("No episodes listed yet.", cx))
                } else {
                    this.child(list)
                }
            })
    }

    /// One episode: clicking it plays, the chevron lists its sources.
    fn episode_row(
        &self,
        index: usize,
        video: Video,
        is_next: bool,
        runtime: Option<u32>,
        cx: &mut Context<Self>,
    ) -> Div {
        let theme = cx.theme();
        let expanded = self.episode.as_deref() == Some(video.id.as_str());
        let starting = self.starting.as_deref() == Some(video.id.as_str());
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
        let item = self.play_item(Some(&video));

        // The player's own duration beats the series' typical one.
        let duration = entry.as_ref().map(|e| e.duration).filter(|d| *d > 0.0);
        let length = match (duration, progress) {
            (Some(total), Some(_)) => Some(fmt_time_left(
                entry.as_ref().map_or(0.0, |e| e.position),
                total,
            )),
            (Some(total), None) => Some(fmt_minutes(((total / 60.0).round() as u32).max(1))),
            (None, _) => runtime.map(fmt_minutes),
        };
        let subtitle = [video.released.as_deref().map(fmt_date), length]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join("  ·  ");
        let group: SharedString = format!("episode-{index}").into();

        let on_disk = match files.len() {
            0 => None,
            1 => Some("On disk".to_string()),
            n => Some(format!("{n} copies")),
        };
        let number = video
            .episode_number()
            .map(|n| n.to_string())
            .unwrap_or_default();
        let title = video
            .title()
            .map(str::to_string)
            .unwrap_or_else(|| video.label());

        let thumb = div()
            .relative()
            .w(px(168.))
            .h(px(94.))
            .flex_none()
            .rounded(theme.radius)
            .overflow_hidden()
            .bg(theme.muted)
            .when_some(video.thumbnail.clone(), |this, url| {
                this.child(img(url).size_full().object_fit(ObjectFit::Cover))
            })
            .child(
                div()
                    .absolute()
                    .inset_0()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(black().alpha(0.35))
                    .map(|this| {
                        if starting {
                            this.child(Spinner::new().color(white()))
                        } else {
                            this.invisible()
                                .group_hover(group.clone(), |this| this.visible())
                                .child(
                                    div()
                                        .size(px(40.))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .rounded_full()
                                        .border_2()
                                        .border_color(white())
                                        .bg(black().alpha(0.5))
                                        .child(
                                            Icon::new(IconName::Play).size_5().text_color(white()),
                                        ),
                                )
                        }
                    }),
            )
            .when_some(progress, |this, progress| {
                this.child(
                    div()
                        .absolute()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .h(px(4.))
                        .bg(black().alpha(0.55))
                        .child(div().h_full().w(relative(progress)).bg(theme.primary)),
                )
            });

        let watched_item = item.clone();
        let video_id = video.id.clone();
        let play_item = item.clone();
        // With nothing to play from, a click goes where a source is added.
        let no_source = files.is_empty() && self.state.read(cx).sources().is_empty();
        let row = h_flex()
            .id(eid(format!("episode-{index}")))
            .group(group.clone())
            .w_full()
            .gap_4()
            .px_3()
            .py_3()
            .items_center()
            .rounded(theme.radius_lg)
            .cursor_pointer()
            .border_b_1()
            .border_color(theme.border.alpha(0.5))
            .when(expanded || is_next, |this| this.bg(theme.list_active))
            .hover(|this| this.bg(theme.list_hover))
            .on_click(cx.listener(move |this, _, window, cx| {
                if no_source {
                    cx.emit(NavEvent::OpenSettings);
                } else {
                    this.quick_play(play_item.clone(), window, cx)
                }
            }))
            .child(
                div()
                    .w(px(28.))
                    .flex_none()
                    .text_xl()
                    .text_color(theme.muted_foreground)
                    .text_center()
                    .child(number),
            )
            .child(thumb)
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(div().text_base().font_semibold().truncate().child(title))
                            .when(is_next, |this| {
                                this.child(Tag::primary().small().child(if progress.is_some() {
                                    "Continue"
                                } else {
                                    "Next up"
                                }))
                            })
                            .when_some(on_disk, |this, label| {
                                this.child(Tag::secondary().small().child(label))
                            }),
                    )
                    .when(!subtitle.is_empty(), |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(subtitle),
                        )
                    })
                    .when_some(
                        video.overview().filter(|o| !o.is_empty()),
                        |this, overview| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .line_clamp(2)
                                    .text_ellipsis()
                                    .child(overview.to_string()),
                            )
                        },
                    ),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap_1()
                    .child(
                        Button::new(eid(format!("episode-{index}-watched")))
                            .ghost()
                            .small()
                            .icon(if watched {
                                IconName::CircleCheck
                            } else {
                                IconName::Circle
                            })
                            .tooltip(if watched {
                                "Watched. Click to unmark"
                            } else {
                                "Mark watched"
                            })
                            .when(watched, |this| this.text_color(theme.success))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                // The row behind plays on click; this doesn't.
                                cx.stop_propagation();
                                this.set_watched(watched_item.clone(), !watched, cx)
                            })),
                    )
                    .child(
                        Button::new(eid(format!("episode-{index}-sources")))
                            .ghost()
                            .small()
                            .icon(if expanded {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .tooltip("Choose a source")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                this.select_episode(video_id.clone(), cx)
                            })),
                    ),
            );

        if !expanded {
            return div().w_full().child(row);
        }
        v_flex().w_full().child(row).child(
            div()
                .ml(px(44.))
                .pl_4()
                .py_4()
                .border_l_2()
                .border_color(theme.primary)
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let header = self.header(window, cx);
        let body = self.body(window, cx);
        page("detail")
            .p_0()
            .gap_0()
            .child(header)
            .child(v_flex().gap_6().px_6().pt_2().pb_8().child(body))
    }
}
