//! The lower half of a details sheet: a series' episodes by season, a
//! title's files when it matched nothing online, and the full list of
//! sources (copies on this computer first, then addon streams) for when you
//! want to choose.

use rune::{Color, Cursor, El, Icon, SurfaceFit, col, el, icon, row, text};

use super::detail::Detail;
use super::format::{fmt_bytes, fmt_clock, fmt_date, fmt_minutes, fmt_time_left, runtime_minutes};
use super::images::px;
use super::play::{local_item, next_up, play_item};
use super::theme::{t, ty};
use super::widgets::{
    Button, Size, Tone, Variant, busy, cover, dotted, error_text, heading, muted, progress,
    spinner, tag,
};
use super::{Action, App, Load, icons};
use crate::addons::{Meta, Video};
use crate::library::LocalFile;
use crate::state::{FoundStream, StreamResults};
use crate::watch::PlayItem;

pub fn episodes(s: &App, d: &Detail, meta: &Meta, scale: f32) -> El {
    let seasons = meta.seasons();
    let season = d.season.or_else(|| seasons.first().copied());
    let next = next_up(meta, &s.core.watch).map(|v| v.id.clone());
    // Cinemeta gives one typical episode length per series.
    let runtime = meta.runtime.as_deref().and_then(runtime_minutes);

    let mut picker = row().key("seasons").gap(8.0).flex_wrap().role("tablist");
    for &n in &seasons {
        picker = picker.child(season_pill(d.id, n, season == Some(n)));
    }
    let list = match season {
        None => muted("No episodes listed yet."),
        Some(n) => col().key(format!("season-{n}")).w_full().gap(4.0).children(
            meta.episodes(n).into_iter().map(|video| {
                episode_row(
                    s,
                    d,
                    video,
                    next.as_deref() == Some(&video.id),
                    runtime,
                    scale,
                )
            }),
        ),
    };
    col()
        .key("episodes")
        .w_full()
        .gap(16.0)
        .pt(12.0)
        .child(
            row()
                .gap(24.0)
                .center_y()
                .flex_wrap()
                .child(heading("Episodes"))
                .child(picker),
        )
        .child(list)
}

fn season_pill(id: u64, n: u32, selected: bool) -> El {
    let th = t();
    let label = if n == 0 {
        "Specials".to_string()
    } else {
        format!("Season {n}")
    };
    let (fill, ink) = if selected {
        (th.text, th.bg)
    } else {
        (th.raised2, th.text)
    };
    row()
        .key(format!("season-{n}"))
        .h(32.0)
        .px(14.0)
        .center()
        .radius(16.0)
        .fill(fill)
        .hover(if selected {
            Color::TRANSPARENT
        } else {
            th.hover
        })
        .cursor(Cursor::Pointer)
        .role("tab")
        .checked(selected)
        .focusable(true)
        .motion(super::theme::motion::QUICK)
        .on_press(move |s: &mut App| s.pick_season(id, n))
        .child(text(label).font(ty::SMALL_STRONG).color(ink))
}

/// One episode: clicking it plays, the chevron lists its sources.
fn episode_row(
    s: &App,
    d: &Detail,
    video: &Video,
    is_next: bool,
    runtime: Option<u32>,
    scale: f32,
) -> El {
    let th = t();
    let id = d.id;
    let expanded = d.episode.as_deref() == Some(video.id.as_str());
    let starting = s.is_starting(&video.id);
    let entry = s.core.watch.get(&video.id);
    let watched = entry.is_some_and(|e| e.watched);
    let progress_done = entry.map(|e| e.progress()).filter(|p| *p > 0.0 && !watched);
    let files = s.core.local_files(&d.target.id, Some(&video.id));
    let item = play_item(&d.target, Some(video));

    // The player's own duration beats the series' typical one.
    let duration = entry.map(|e| e.duration).filter(|d| *d > 0.0);
    let length = match (duration, progress_done) {
        (Some(total), Some(_)) => Some(fmt_time_left(entry.map_or(0.0, |e| e.position), total)),
        (Some(total), None) => Some(fmt_minutes(((total / 60.0).round() as u32).max(1))),
        (None, _) => runtime.map(fmt_minutes),
    };
    let subtitle = dotted([video.released.as_deref().map(fmt_date), length]);

    let (w, h) = (176.0, 99.0);
    let mut thumb = el()
        .key("still")
        .w(w)
        .h(h)
        .radius(8.0)
        .clip_rounded(8.0)
        .fill(th.raised2);
    if let Some(url) = &video.thumbnail {
        let art = s.images.get(url, px([w, h], scale));
        if !art.failed {
            thumb = thumb.child(
                el().w_full()
                    .h_full()
                    .pixels_fit(&art.pixels, SurfaceFit::Fill),
            );
        }
    }
    thumb = if starting {
        thumb.child(
            cover()
                .center()
                .fill(Color::BLACK.alpha(0.55))
                .child(spinner(26.0, Color::WHITE)),
        )
    } else {
        thumb.child(
            cover()
                .center()
                .fill(Color::BLACK.alpha(0.35))
                .group_opacity(0.0, 1.0)
                .child(
                    el().size(42.0, 42.0)
                        .radius(21.0)
                        .center()
                        .fill(Color::BLACK.alpha(0.5))
                        .border(2.0, Color::WHITE)
                        .child(icon(&icons::PLAY, 20.0).color(Color::WHITE)),
                ),
        )
    };
    if let Some(value) = progress_done {
        thumb = thumb.child(
            el().pin(Some(0.0), None, Some(0.0), Some(0.0))
                .child(progress(value, 4.0, Color::BLACK.alpha(0.6), th.accent)),
        );
    }

    let mut title_line = row().gap(8.0).center_y().child(
        text(
            video
                .title()
                .map(str::to_string)
                .unwrap_or_else(|| video.label()),
        )
        .font(ty::SUBHEAD)
        .color(th.text)
        .truncate()
        .shrink(),
    );
    if is_next {
        title_line = title_line.child(tag(
            if progress_done.is_some() {
                "Continue"
            } else {
                "Next up"
            },
            Tone::Accent,
        ));
    }
    match files.len() {
        0 => {}
        1 => title_line = title_line.child(tag("On disk", Tone::Neutral)),
        n => title_line = title_line.child(tag(format!("{n} copies"), Tone::Neutral)),
    }
    let mut details = col().gap(4.0).grow(1.0).shrink().child(title_line);
    if !subtitle.is_empty() {
        details = details.child(text(subtitle).font(ty::CAPTION).color(th.muted));
    }
    if let Some(overview) = video.overview().filter(|o| !o.is_empty()) {
        details = details.child(
            text(overview.to_string())
                .font(ty::SMALL)
                .color(th.muted)
                .max_lines(2),
        );
    }

    // With nothing to play from, a click goes where a source is added.
    let no_source = files.is_empty() && s.core.sources().is_empty();
    let play = item.clone();
    let video_id = video.id.clone();
    let toggle = item.clone();
    let row_el = row()
        .key(format!("ep-{}", video.id))
        .w_full()
        .gap(16.0)
        .padding(10.0)
        .center_y()
        .radius(12.0)
        .fill(if expanded || is_next {
            th.raised
        } else {
            Color::TRANSPARENT
        })
        .hover(th.hover)
        .hover_group()
        .cursor(Cursor::Pointer)
        .role("button")
        .focusable(true)
        .on_press_tx(move |s: &mut App, tx| match no_source {
            true => s.act(Action::AddSource, tx),
            false => s.quick_play(id, play.clone()),
        })
        .child(
            text(
                video
                    .episode_number()
                    .map(|n| n.to_string())
                    .unwrap_or_default(),
            )
            .font(ty::HEADING)
            .color(th.muted)
            .w(30.0)
            .align(rune::Align::Center),
        )
        .child(thumb)
        .child(details)
        .child(
            row()
                .gap(4.0)
                .child(
                    Button::icon(
                        if watched {
                            &icons::CIRCLE_CHECK
                        } else {
                            &icons::CIRCLE
                        },
                        if watched {
                            "Watched. Click to unmark"
                        } else {
                            "Mark watched"
                        },
                    )
                    .variant(Variant::Ghost)
                    .ink(if watched { th.success } else { th.muted })
                    .on_press(move |s: &mut App, _| s.set_watched(&toggle, !watched))
                    .build(),
                )
                .child(
                    Button::icon(
                        if expanded {
                            &icons::CHEVRON_DOWN
                        } else {
                            &icons::CHEVRON_RIGHT
                        },
                        "Choose a source",
                    )
                    .variant(Variant::Ghost)
                    .ink(th.muted)
                    .on_press(move |s: &mut App, _| s.select_episode(id, video_id.clone()))
                    .build(),
                ),
        );
    if !expanded {
        return row_el;
    }
    col()
        .key(format!("ep-{}-open", video.id))
        .w_full()
        .gap(4.0)
        .child(row_el)
        .child(
            row()
                .w_full()
                .pl(56.0)
                .pt(8.0)
                .pb(12.0)
                .child(el().w(3.0).radius(1.5).fill(th.accent).self_stretch())
                .child(col().grow(1.0).shrink().pl(18.0).child(sources_section(
                    s,
                    d,
                    &files,
                    Some(&video.id),
                    item,
                ))),
        )
}

/// A movie's sources, folded out under the hero.
pub fn movie_sources(s: &App, d: &Detail, meta: &Meta) -> El {
    let files = s.core.local_files(&meta.id, None);
    col().w_full().pt(16.0).child(sources_section(
        s,
        d,
        &files,
        Some(&meta.id),
        play_item(&d.target, None),
    ))
}

/// A title that matched nothing online: its own files, and nothing else.
pub fn local_body(s: &App, d: &Detail) -> El {
    let videos = s
        .core
        .library_title(&d.target.id)
        .map(|title| title.videos())
        .unwrap_or_default();
    let note = muted(
        "This title didn't match anything online, so there are no details or streams. Its files are below.",
    );
    let mut out = col().key("local").w_full().gap(24.0).pt(12.0).child(note);
    if videos.is_empty() {
        return out.child(muted("No files found for this title."));
    }
    if d.target.kind != "series" {
        let video = &videos[0];
        return out.child(sources_section(
            s,
            d,
            &video.versions,
            None,
            local_item(&d.target, video),
        ));
    }
    let mut season = None;
    for (i, video) in videos.iter().enumerate() {
        if i == 0 || video.season != season {
            season = video.season;
            out = out.child(heading(match season {
                Some(0) => "Specials".to_string(),
                Some(n) => format!("Season {n}"),
                None => "Episodes".to_string(),
            }));
        }
        let item = local_item(&d.target, video);
        let label = item
            .episode_label
            .clone()
            .unwrap_or_else(|| video.versions[0].file_name());
        out = out.child(
            col()
                .key(format!("local-{i}"))
                .w_full()
                .gap(12.0)
                .child(text(label).font(ty::LABEL).color(t().text))
                .child(local_actions(s, &video.versions, item.clone()))
                .child(local_rows(s, &video.versions, &item)),
        );
    }
    out
}

/// Play (the best local copy, resuming where the history says) and the
/// watched toggle.
fn local_actions(s: &App, files: &[LocalFile], item: PlayItem) -> El {
    let entry = s.core.watch.get(&item.video_id);
    let watched = entry.is_some_and(|e| e.watched);
    let resume = entry.and_then(|e| e.resume_position());
    let mut out = row().gap(10.0).center_y();
    if let Some(best) = files.first().cloned() {
        let key = best.path.to_string_lossy().into_owned();
        let starting = s.is_starting(&key);
        let label = match (starting, resume) {
            (true, _) => "Starting…".to_string(),
            (false, Some(position)) => format!("Resume from {}", fmt_clock(position)),
            (false, None) => "Play".to_string(),
        };
        let play = item.clone();
        out = out.child(
            Button::new(label)
                .with_icon(&icons::PLAY)
                .variant(Variant::Accent)
                .busy(starting)
                .disabled(s.starting.is_some() && !starting)
                .on_press(move |s: &mut App, _| s.play_file(best.clone(), play.clone()))
                .build(),
        );
    }
    out.child(
        Button::new(if watched { "Watched" } else { "Mark watched" })
            .with_icon(if watched {
                &icons::CIRCLE_CHECK
            } else {
                &icons::CIRCLE
            })
            .on_press(move |s: &mut App, _| s.set_watched(&item, !watched))
            .build(),
    )
}

/// Everything that can play one video: copies on this computer, then addon
/// streams when `stream_video_id` is given.
fn sources_section(
    s: &App,
    d: &Detail,
    files: &[LocalFile],
    stream_video_id: Option<&str>,
    item: PlayItem,
) -> El {
    let mut list = col()
        .w_full()
        .gap(8.0)
        .children((!files.is_empty()).then(|| local_rows(s, files, &item)));
    if let Some(video_id) = stream_video_id {
        let results = d
            .streams
            .as_ref()
            .filter(|(id, _)| id == video_id)
            .map(|(_, r)| r);
        list = list.child(match results {
            None | Some(Load::Loading) => {
                busy("Looking for streams… torrents can take a few seconds.")
            }
            Some(Load::Failed(err)) => error_text(format!("Couldn't look for streams: {err}")),
            Some(Load::Ready(results)) => stream_list(s, results, files.is_empty(), &item),
        });
    }
    col()
        .key("sources")
        .w_full()
        .gap(12.0)
        .child(text("Sources").font(ty::SUBHEAD).color(t().text))
        .child(list)
}

/// One row per local copy, best first.
fn local_rows(s: &App, files: &[LocalFile], item: &PlayItem) -> El {
    let several = files.len() > 1;
    col()
        .w_full()
        .gap(8.0)
        .children(files.iter().enumerate().map(|(i, file)| {
            let th = t();
            let quality = match file.quality.label() {
                label if label.is_empty() => "Unknown quality".to_string(),
                label => label,
            };
            let folder = file
                .path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
            let mut title = row()
                .gap(8.0)
                .center_y()
                .child(text("This computer").font(ty::SMALL_STRONG).color(th.text))
                .child(tag(quality, Tone::Neutral));
            if several && i == 0 {
                title = title.child(tag("Best", Tone::Success));
            }
            let detail = col()
                .gap(3.0)
                .child(title)
                .child(
                    text(format!("{}  ·  {}", file.file_name(), fmt_bytes(file.size)))
                        .font(ty::CAPTION)
                        .color(th.text.alpha(0.85))
                        .truncate(),
                )
                .child(
                    text(folder)
                        .font(ty::CAPTION)
                        .color(th.muted)
                        .truncate_middle(),
                );
            let (file, item) = (file.clone(), item.clone());
            let key = file.path.to_string_lossy().into_owned();
            source_row(
                s,
                format!("file-{i}"),
                &key,
                &icons::HARD_DRIVE,
                detail,
                move |s| s.play_file(file.clone(), item.clone()),
            )
        }))
}

fn stream_list(s: &App, results: &StreamResults, no_local: bool, item: &PlayItem) -> El {
    if results.asked == 0 {
        return muted("Add a source in Settings to find streams.");
    }
    let th = t();
    let mut out = col().w_full().gap(8.0);
    let mut any = false;
    for (i, found) in results.playable().enumerate() {
        any = true;
        out = out.child(stream_row(s, i, found, item));
    }
    if !any && no_local {
        out = out.child(muted("No streams found for this video."));
    }
    out.children(results.failures.iter().map(|(source, err)| {
        text(format!("{source} didn't answer: {err}"))
            .font(ty::CAPTION)
            .color(th.muted)
            .truncate()
    }))
}

fn stream_row(s: &App, i: usize, found: &FoundStream, item: &PlayItem) -> El {
    let th = t();
    let mut title = row().gap(8.0).center_y().child(
        text(found.source.clone())
            .font(ty::SMALL_STRONG)
            .color(th.text)
            .truncate()
            .shrink(),
    );
    if let Some(quality) = found.stream.quality() {
        title = title.child(tag(quality, Tone::Neutral));
    }
    title = title.child(if found.stream.is_torrent() {
        tag("Torrent", Tone::Warning)
    } else {
        tag("Direct", Tone::Success)
    });
    let lines = found.stream.text();
    let detail = col().gap(3.0).child(title).children(
        lines
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(|line| {
                text(line.to_string())
                    .font(ty::CAPTION)
                    .color(th.muted)
                    .truncate()
            }),
    );
    let key = found.stream.dedupe_key();
    let (found, item) = (found.clone(), item.clone());
    source_row(
        s,
        format!("stream-{i}"),
        &key,
        &icons::GLOBE,
        detail,
        move |s| s.play_stream(found.clone(), item.clone()),
    )
}

/// The shared shape of a source: an icon, the details, and Play.
fn source_row(
    s: &App,
    key: String,
    starting_key: &str,
    glyph: &'static Icon,
    detail: El,
    play: impl Fn(&mut App) + 'static,
) -> El {
    let th = t();
    let starting = s.is_starting(starting_key);
    row()
        .key(key)
        .w_full()
        .gap(16.0)
        .px(16.0)
        .py(12.0)
        .center_y()
        .radius(12.0)
        .fill(th.raised)
        .border(1.0, th.line)
        .child(icon(glyph, 18.0).color(th.muted))
        .child(detail.grow(1.0).shrink())
        .child(
            Button::new(if starting { "Starting…" } else { "Play" })
                .with_icon(&icons::PLAY)
                .variant(Variant::Accent)
                .size(Size::Small)
                .busy(starting)
                .disabled(s.starting.is_some() && !starting)
                .on_press(move |s: &mut App, _| play(s))
                .build(),
        )
}
