//! Details: one title in a sheet over the page, under its artwork, with one
//! Play that picks the best source and resumes (for a series, the next-up
//! episode). The page stays where it was underneath; Esc or a click outside
//! goes back. "More like this" opens another sheet on top.

use rune::{Color, Cx, El, ScrollSpec, SurfaceFit, Transition, Tx, col, el, row, text};

use super::format::{fmt_runtime, fmt_time_left, short_episode};
use super::home::{backdrop_shade, meta_strip, shelf, title_art};
use super::images::px;
use super::play::{next_up, play_item, play_label};
use super::poster::skeleton_strip;
use super::ratings::badges;
use super::theme::{float_shadow, motion, space, t, ty};
use super::widgets::{Button, Size, Variant, cover, dotted, error_text, progress, skeleton};
use super::{Action, App, DetailTarget, Load, icons};
use crate::addons::Meta;
use crate::state::StreamResults;
use crate::watch::PlayItem;

/// Descriptions longer than this are cut in the hero, so they're shown in
/// full under it too.
const LONG_DESCRIPTION: usize = 320;

pub struct Detail {
    pub id: u64,
    pub target: DetailTarget,
    pub info: Info,
    pub season: Option<u32>,
    /// The episode whose sources are folded out, for a series.
    pub episode: Option<String>,
    /// Streams for one video id, once asked for.
    pub streams: Option<(String, Load<StreamResults>)>,
    /// Whether a movie's source list is folded out.
    pub show_sources: bool,
    /// Popular titles sharing the first genre, once asked for.
    pub similar: Option<Load<Vec<Meta>>>,
    /// An episode to fold out as soon as the metadata arrives.
    pub preselect: Option<String>,
}

/// Where the title's metadata stands. Local-only titles never load any.
pub enum Info {
    Local,
    Loading,
    Ready(Box<Meta>),
    Failed(String),
}

impl Detail {
    pub fn meta(&self) -> Option<&Meta> {
        match &self.info {
            Info::Ready(meta) => Some(meta),
            _ => None,
        }
    }

    /// The streams already found for `video_id`, so Play doesn't ask again.
    pub fn known_streams(&self, video_id: &str) -> Option<StreamResults> {
        match &self.streams {
            Some((id, Load::Ready(results))) if id == video_id => Some(results.clone()),
            _ => None,
        }
    }
}

/// What the hero's Play starts.
enum Primary {
    Video(PlayItem),
    /// An unmatched title: its next file, found at click time.
    LocalTitle,
    /// Nothing on this computer and no sources: offer to add one.
    NoSource,
}

impl App {
    pub fn open_detail(&mut self, target: DetailTarget) {
        let id = self.next_id();
        let local = target.is_local();
        if !local {
            self.bg.run(
                self.core.meta(&target.kind, &target.id),
                move |s, meta, _| match meta {
                    Ok(meta) => s.meta_loaded(id, meta),
                    Err(err) => {
                        if let Some(d) = s.detail_mut(id) {
                            d.info = Info::Failed(format!("{err:#}"));
                        }
                    }
                },
            );
        }
        self.nav.details.push(Detail {
            id,
            info: if local { Info::Local } else { Info::Loading },
            target,
            season: None,
            episode: None,
            streams: None,
            show_sources: false,
            similar: None,
            preselect: None,
        });
    }

    /// A sheet by id, showing or set aside; `None` once it's closed.
    pub fn detail_mut(&mut self, id: u64) -> Option<&mut Detail> {
        self.nav
            .details
            .iter_mut()
            .chain(&mut self.nav.parked)
            .find(|d| d.id == id)
    }

    fn meta_loaded(&mut self, id: u64, meta: Meta) {
        if self.detail_mut(id).is_none() {
            return;
        }
        let series = meta.kind == "series";
        // A series opens on the season you're watching.
        let season = series
            .then(|| {
                next_up(&meta, &self.core.watch)
                    .and_then(|v| v.season)
                    .or_else(|| meta.seasons().first().copied())
            })
            .flatten();
        if !series {
            self.load_streams(id, meta.id.clone());
        }
        self.load_ratings(&meta.id);
        self.load_similar(id, &meta);
        let Some(d) = self.detail_mut(id) else { return };
        d.target.name = meta.name.clone();
        d.target.poster = meta.poster.clone();
        d.season = season;
        d.info = Info::Ready(Box::new(meta));
        if let Some(video_id) = d.preselect.take() {
            self.select_episode(id, video_id);
        }
    }

    fn load_streams(&mut self, id: u64, video_id: String) {
        let Some(d) = self.detail_mut(id) else { return };
        d.streams = Some((video_id.clone(), Load::Loading));
        let kind = d.target.kind.clone();
        self.bg
            .run(self.core.streams(&kind, &video_id), move |s, results, _| {
                if let Some(d) = s.detail_mut(id)
                    && d.streams
                        .as_ref()
                        .is_some_and(|(asked, _)| *asked == video_id)
                {
                    d.streams = Some((video_id, Load::from_result(results)));
                }
            });
    }

    fn load_similar(&mut self, id: u64, meta: &Meta) {
        let Some(genre) = meta.genres().first() else {
            return;
        };
        let own = meta.id.clone();
        self.bg
            .run(self.core.genre(&meta.kind, genre), move |s, items, _| {
                let items =
                    items.map(|items| items.into_iter().filter(|m| m.id != own).take(20).collect());
                if let Some(d) = s.detail_mut(id) {
                    d.similar = Some(Load::from_result(items));
                }
            });
        if let Some(d) = self.detail_mut(id) {
            d.similar = Some(Load::Loading);
        }
    }

    /// Refreshes a title's review scores, at most once at a time.
    pub fn load_ratings(&mut self, imdb_id: &str) {
        let Some((key, request)) = self.core.ratings_request(imdb_id) else {
            return;
        };
        let imdb_id = imdb_id.to_string();
        self.bg.run(request, move |s, result, _| {
            s.core.ratings_fetched(&imdb_id, key, result)
        });
    }

    /// Folds an episode's sources out (and shows its season), or folds them
    /// back in.
    pub fn select_episode(&mut self, id: u64, video_id: String) {
        let Some(d) = self.detail_mut(id) else { return };
        if let Some(season) = d
            .meta()
            .and_then(|m| m.videos.iter().find(|v| v.id == video_id))
            .and_then(|v| v.season)
        {
            d.season = Some(season);
        }
        if d.episode.as_deref() == Some(&video_id) {
            d.episode = None;
            d.streams = None;
        } else {
            d.episode = Some(video_id.clone());
            self.load_streams(id, video_id);
        }
    }

    pub fn pick_season(&mut self, id: u64, season: u32) {
        if let Some(d) = self.detail_mut(id) {
            d.season = Some(season);
            d.episode = None;
            d.streams = None;
        }
    }

    /// Plays one video of a sheet with the best source there is.
    pub fn quick_play(&mut self, id: u64, item: PlayItem) {
        let known = self
            .detail_mut(id)
            .and_then(|d| d.known_streams(&item.video_id));
        self.play_video(item, known);
    }

    /// What the hero's Play starts, and what its button says: the movie,
    /// the series' next-up episode, or an unmatched title's next file.
    /// `None` until there's something to play.
    fn primary(&self, d: &Detail) -> Option<(Primary, String)> {
        let core = &self.core;
        let nothing_to_play = !d.target.is_local()
            && core.sources().is_empty()
            && core.local_files(&d.target.id, None).is_empty();
        match &d.info {
            Info::Ready(_) if nothing_to_play => {
                Some((Primary::NoSource, "Add a source to play".to_string()))
            }
            Info::Ready(meta) if meta.kind == "series" => {
                let video = next_up(meta, &core.watch)?;
                let short = short_episode(&video.label());
                let label = play_label(&core.watch, &video.id, Some(&short));
                Some((Primary::Video(play_item(&d.target, Some(video))), label))
            }
            Info::Ready(_) => Some((
                Primary::Video(play_item(&d.target, None)),
                play_label(&core.watch, &d.target.id, None),
            )),
            Info::Local => {
                let title = core.library_title(&d.target.id)?;
                (!title.files.is_empty()).then(|| (Primary::LocalTitle, "Play".to_string()))
            }
            Info::Loading | Info::Failed(_) => None,
        }
    }

    fn play_primary(&mut self, id: u64, tx: &mut Tx) {
        let Some(d) = self.nav.details.iter().find(|d| d.id == id) else {
            return;
        };
        let target = d.target.clone();
        match self.primary(d) {
            Some((Primary::Video(item), _)) => self.quick_play(id, item),
            Some((Primary::NoSource, _)) => self.act(Action::AddSource, tx),
            Some((Primary::LocalTitle, _)) => self.play_title(target),
            None => {}
        }
    }
}

/// The sheet: a panel over a dimmed, blurred page.
pub fn sheet(s: &App, d: &Detail, cx: &mut Cx<App>) -> El {
    let th = t();
    let [window_w, _] = cx.layout_size("app").unwrap_or([1280.0, 800.0]);
    let width = (window_w - 96.0).clamp(720.0, 1100.0);
    let scale = cx.env().scale;

    let mut content = col().w_full().child(hero(s, d, scale)).child(
        col()
            .key("main")
            .w_full()
            .px(space::PAGE)
            .pt(8.0)
            .pb(8.0)
            .child(body(s, d, scale)),
    );
    if let Some(similar) = similar(s, d, scale) {
        content = content.child(col().w_full().pt(space::SECTION).child(similar));
    }
    if let Some(about) = d.meta().and_then(about) {
        content = content.child(about.px(space::PAGE).pt(space::SECTION));
    }

    let radii = [16.0, 16.0, 0.0, 0.0];
    let mut panel = el()
        .key(format!("sheet-{}", d.id))
        .w(width)
        .h_full()
        .corners(radii)
        .clip_corners(radii)
        .fill(th.bg)
        .border(1.0, th.line)
        .box_shadow(float_shadow())
        .role("dialog")
        .label(d.target.name.clone())
        .modal(true, |s: &mut App| {
            s.nav.details.pop();
        })
        .transition(
            Transition::offset(0.0, 48.0)
                .combined(Transition::opacity())
                .springs(rune::SpringPair::new(motion::PANEL, motion::QUICK)),
        )
        .child(
            col()
                .key(format!("detail-{}", d.id))
                .w_full()
                .h_full()
                .scroll(ScrollSpec {
                    stick_bottom: false,
                })
                .child(content.pb(64.0)),
        )
        .child(
            Button::icon(&icons::CLOSE, "Close")
                .variant(Variant::Glass)
                .round()
                .key("close")
                .on_press(|s: &mut App, _| s.nav.details.clear())
                .build()
                .pin(None, Some(16.0), Some(16.0), None),
        );
    if s.nav.details.len() > 1 {
        panel = panel.child(
            Button::icon(&icons::ARROW_LEFT, "Back")
                .variant(Variant::Glass)
                .round()
                .key("back")
                .on_press(super::on(Action::Back))
                .build()
                .pin(Some(16.0), Some(16.0), None, None),
        );
    }
    col()
        .key("sheet-layer")
        .pin(Some(0.0), Some(0.0), Some(0.0), Some(0.0))
        .items_center()
        .pt(28.0)
        .fill(Color::BLACK.alpha(0.55))
        .backdrop_blur(10.0)
        .transition(Transition::opacity())
        .child(panel)
}

/// The title under its artwork: logo or name, scores, facts, the story,
/// how far you got, and Play.
fn hero(s: &App, d: &Detail, scale: f32) -> El {
    let th = t();
    let meta = d.meta();
    let backdrop = meta.and_then(|m| m.background.as_deref());
    let height = if d.target.is_local() { 340.0 } else { 470.0 };

    let mut facts = Vec::new();
    if let Some(meta) = meta {
        facts.push(meta.year_label().map(str::to_string));
        // A series' runtime is per episode, so it's shown on the episodes.
        if meta.kind == "series" {
            facts.push(Some(
                match meta.seasons().iter().filter(|&&s| s > 0).count() {
                    0 => "Series".to_string(),
                    1 => "1 season".to_string(),
                    n => format!("{n} seasons"),
                },
            ));
        } else {
            facts.push(meta.runtime.as_deref().map(fmt_runtime));
        }
    }
    let scores = (!d.target.is_local())
        .then(|| {
            badges(
                s.core.ratings(&d.target.id).as_ref(),
                meta.and_then(|m| m.imdb_rating.as_deref()),
                false,
            )
        })
        .flatten();

    let title = match meta {
        Some(meta) if backdrop.is_some() => title_art(s, meta, 400.0, 110.0, ty::DISPLAY),
        _ => text(d.target.name.clone())
            .font(ty::DISPLAY)
            .color(th.text)
            .max_lines(2),
    };
    let mut info = col()
        .gap(12.0)
        .max_w(600.0)
        .grow(1.0)
        .shrink()
        .child(title)
        .child(
            row().gap(16.0).center_y().children(scores).child(
                text(dotted(facts))
                    .font(ty::SMALL_STRONG)
                    .color(th.text.alpha(0.88)),
            ),
        );
    if let Some(meta) = meta {
        let genres = meta.genres().iter().take(4).cloned().collect::<Vec<_>>();
        if !genres.is_empty() {
            info = info.child(text(genres.join("  •  ")).font(ty::SMALL).color(th.muted));
        }
        if let Some(about) = &meta.description {
            info = info.child(
                text(about.clone())
                    .font(ty::BODY)
                    .color(th.text.alpha(0.9))
                    .max_lines(4),
            );
        }
    }
    info = info.children(resume_line(s, d)).child(actions(s, d));

    let mut stage = el().key("hero").w_full().h(height).fill(th.raised);
    if let Some(url) = backdrop {
        stage = stage.child(cover().pixels_fit(&s.images.get(url, None).pixels, SurfaceFit::Fill));
    }
    // The poster stands in for missing backdrop art.
    let poster = backdrop.is_none().then(|| {
        let (w, h) = (170.0, 255.0);
        let art = d
            .target
            .poster
            .as_deref()
            .map(|url| s.images.get(url, px([w, h], scale)));
        let mut tile = el()
            .w(w)
            .h(h)
            .radius(10.0)
            .clip_rounded(10.0)
            .fill(th.raised2);
        if let Some(art) = art.filter(|a| !a.failed) {
            tile = tile.child(
                el().w_full()
                    .h_full()
                    .pixels_fit(&art.pixels, SurfaceFit::Fill),
            );
        }
        tile
    });
    stage.child(backdrop_shade(th.bg)).child(
        row()
            .pin(Some(space::PAGE), None, Some(space::PAGE), Some(24.0))
            .gap(28.0)
            .items_end()
            .children(poster)
            .child(info),
    )
}

/// How far into the movie, or the series' next-up episode, playback got:
/// a bar and `S1 E3 · 29m left`.
fn resume_line(s: &App, d: &Detail) -> Option<El> {
    let meta = d.meta()?;
    let (video_id, episode) = if meta.kind == "series" {
        let video = next_up(meta, &s.core.watch)?;
        (video.id.clone(), Some(short_episode(&video.label())))
    } else {
        (d.target.id.clone(), None)
    };
    let entry = s.core.watch.get(&video_id)?;
    if entry.watched || entry.duration <= 0.0 || entry.position <= 0.0 {
        return None;
    }
    let left = fmt_time_left(entry.position, entry.duration);
    Some(
        row()
            .gap(12.0)
            .center_y()
            .child(el().w(180.0).child(progress(
                entry.progress(),
                4.0,
                Color::WHITE.alpha(0.25),
                t().accent,
            )))
            .child(
                text(dotted([episode, Some(left)]))
                    .font(ty::SMALL)
                    .color(t().text.alpha(0.85)),
            ),
    )
}

/// Play (the best source, resuming), then for a movie the watched toggle
/// and the source list.
fn actions(s: &App, d: &Detail) -> El {
    let id = d.id;
    let primary = s.primary(d);
    let busy = match (&primary, &s.starting) {
        (Some((Primary::Video(item), _)), Some(key)) => *key == item.video_id,
        (Some((Primary::LocalTitle, _)), Some(key)) => *key == d.target.id,
        _ => false,
    };
    let mut out = row().key("actions").gap(12.0).pt(8.0).center_y();
    match &primary {
        Some((kind, label)) => {
            let no_source = matches!(kind, Primary::NoSource);
            out = out.child(
                Button::new(if busy { "Starting…" } else { label.as_str() })
                    .with_icon(if no_source {
                        &icons::PLUS
                    } else {
                        &icons::PLAY
                    })
                    .variant(if no_source {
                        Variant::Glass
                    } else {
                        Variant::Light
                    })
                    .size(Size::Large)
                    .busy(busy)
                    .key("play")
                    .on_press(move |s: &mut App, tx| s.play_primary(id, tx))
                    .build(),
            );
        }
        None if matches!(d.info, Info::Loading) => out = out.child(skeleton(Some(150.0), 46.0)),
        None => {}
    }
    let movie = d.target.kind != "series";
    if !movie || matches!(d.info, Info::Loading | Info::Failed(_)) {
        return out;
    }
    let item = match &d.info {
        Info::Local => s.core.library_title(&d.target.id).and_then(|t| {
            t.videos()
                .first()
                .map(|v| super::play::local_item(&d.target, v))
        }),
        _ => Some(play_item(&d.target, None)),
    };
    if let Some(item) = item {
        let watched = s.core.watch.get(&item.video_id).is_some_and(|e| e.watched);
        out = out.child(
            Button::new(if watched { "Watched" } else { "Mark watched" })
                .with_icon(if watched {
                    &icons::CIRCLE_CHECK
                } else {
                    &icons::CIRCLE
                })
                .variant(Variant::Glass)
                .size(Size::Large)
                .key("watched")
                .on_press(move |s: &mut App, _| s.set_watched(&item, !watched))
                .build(),
        );
    }
    if !d.target.is_local() && !matches!(primary, Some((Primary::NoSource, _))) {
        let found = match &d.streams {
            Some((_, Load::Ready(results))) => results.playable().count(),
            _ => 0,
        };
        let count = found + s.core.local_files(&d.target.id, None).len();
        out = out.child(
            Button::new(if count == 0 {
                "Sources".to_string()
            } else {
                format!("Sources ({count})")
            })
            .with_icon(if d.show_sources {
                &icons::CHEVRON_DOWN
            } else {
                &icons::CHEVRON_RIGHT
            })
            .variant(Variant::Glass)
            .size(Size::Large)
            .key("sources")
            .on_press(move |s: &mut App, _| {
                if let Some(d) = s.detail_mut(id) {
                    d.show_sources ^= true;
                }
            })
            .build(),
        );
    }
    out
}

fn body(s: &App, d: &Detail, scale: f32) -> El {
    match &d.info {
        Info::Loading => col()
            .w_full()
            .gap(14.0)
            .pt(16.0)
            .child(skeleton(Some(220.0), 22.0))
            .child(skeleton(None, 14.0))
            .child(skeleton(None, 14.0))
            .child(skeleton(Some(420.0), 14.0)),
        Info::Failed(err) => error_text(format!("Couldn't load details: {err}")),
        Info::Local => super::sources::local_body(s, d),
        Info::Ready(meta) if meta.kind == "series" => super::sources::episodes(s, d, meta, scale),
        Info::Ready(meta) => match d.show_sources {
            true => super::sources::movie_sources(s, d, meta),
            false => el(),
        },
    }
}

/// Popular titles in the same genre, Netflix's "More like this".
fn similar(s: &App, d: &Detail, scale: f32) -> Option<El> {
    let body = match d.similar.as_ref()? {
        Load::Loading => skeleton_strip(),
        Load::Ready(items) if !items.is_empty() => {
            meta_strip(s, &format!("similar-{}", d.id), items, scale)
        }
        _ => return None,
    };
    Some(shelf("More like this", body))
}

/// The whole story when the hero had to cut it, and who made it.
fn about(meta: &Meta) -> Option<El> {
    let th = t();
    let long = meta
        .description
        .as_deref()
        .filter(|text| text.len() > LONG_DESCRIPTION);
    let people = |label: &str, names: &[String]| {
        (!names.is_empty()).then(|| {
            row()
                .gap(8.0)
                .items_start()
                .child(text(format!("{label}:")).font(ty::SMALL).color(th.muted))
                .child(
                    text(names.join(", "))
                        .font(ty::SMALL)
                        .color(th.text)
                        .wrap()
                        .grow(1.0)
                        .shrink(),
                )
        })
    };
    if long.is_none() && meta.director.is_empty() && meta.cast.is_empty() {
        return None;
    }
    Some(
        col()
            .key("about")
            .w_full()
            .max_w(900.0)
            .gap(12.0)
            .child(text("About").font(ty::HEADING).color(th.text))
            .children(long.map(|about| {
                text(about.to_string())
                    .font(ty::BODY)
                    .color(th.text.alpha(0.9))
                    .wrap()
            }))
            .children(people("Director", &meta.director))
            .children(people("Cast", &meta.cast)),
    )
}
