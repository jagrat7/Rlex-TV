//! One-click play: start a title or episode without choosing a source.
//!
//! The pick is the best copy on this computer, then the first direct
//! (URL) stream a source offers, then the first torrent. A series without
//! a chosen episode plays its next-up episode: the one in progress, the one
//! after the last finished, or the first.

use anyhow::anyhow;
use gpui_kit::*;

use super::DetailTarget;
use crate::addons::{Meta, Video};
use crate::state::{AppState, FoundStream, StreamResults};
use crate::watch::{PlayItem, WatchEntry, WatchState};

/// The first stream worth playing without asking: direct links start
/// faster than torrents, so they win.
pub fn pick_stream(results: &StreamResults) -> Option<&FoundStream> {
    let playable = || results.streams.iter().filter(|f| f.stream.is_playable());
    playable()
        .find(|f| !f.stream.is_torrent())
        .or_else(|| playable().next())
}

/// Why nothing could be picked, worded for a toast.
pub fn no_stream_reason(results: &StreamResults) -> anyhow::Error {
    if results.asked == 0 {
        anyhow!("Nothing on this computer, and no sources added. Add one in Settings.")
    } else if results.streams.is_empty() && !results.failures.is_empty() {
        anyhow!("Your sources didn't answer. Try again in a moment.")
    } else {
        anyhow!("No source has a stream for this yet.")
    }
}

/// The episode to play next, in airing order (specials last): the latest
/// one in progress, else the one after the latest finished, else the first.
/// In-progress wins so ticking off an old episode doesn't move the pick.
pub fn next_up<'a>(meta: &'a Meta, watch: &WatchState) -> Option<&'a Video> {
    let episodes: Vec<&Video> = meta
        .seasons()
        .into_iter()
        .flat_map(|s| meta.episodes(s))
        .collect();
    let latest = |finished: bool| {
        episodes
            .iter()
            .enumerate()
            .filter_map(|(i, v)| watch.get(&v.id).map(|e| (i, e)))
            .filter(|(_, e)| e.watched == finished && (finished || e.position > 0.0))
            .max_by_key(|(_, e)| e.updated)
            .map(|(i, _)| i)
    };
    if let Some(index) = latest(false) {
        return Some(episodes[index]);
    }
    match latest(true) {
        // Past the last episode, start the show again.
        Some(index) => episodes.get(index + 1).or(episodes.first()).copied(),
        None => episodes.first().copied(),
    }
}

/// What a Play button for `video` should say: Resume at a position, or Play.
pub fn play_label(state: &AppState, video_id: &str, episode: Option<&str>) -> String {
    let resume = state
        .watch_entry(video_id)
        .and_then(WatchEntry::resume_position);
    match (resume, episode) {
        (Some(_), Some(ep)) => format!("Resume {ep}"),
        (Some(_), None) => "Resume".to_string(),
        (None, Some(ep)) => format!("Play {ep}"),
        (None, None) => "Play".to_string(),
    }
}

/// The history entry for one video of a title with metadata.
pub fn play_item(target: &DetailTarget, video: Option<&Video>) -> PlayItem {
    let (video_id, episode_label) = match video {
        Some(video) => (video.id.clone(), Some(video.label())),
        None => (target.id.clone(), None),
    };
    PlayItem {
        meta_id: target.id.clone(),
        kind: target.kind.clone(),
        name: target.name.clone(),
        poster: target.poster.clone(),
        video_id,
        episode_label,
    }
}

pub fn item_from_entry(entry: &WatchEntry) -> PlayItem {
    PlayItem {
        meta_id: entry.meta_id.clone(),
        kind: entry.kind.clone(),
        name: entry.name.clone(),
        poster: entry.poster.clone(),
        video_id: entry.video_id.clone(),
        episode_label: entry.episode_label.clone(),
    }
}

/// Plays one video: a local copy if there is one, otherwise a stream from
/// `known` (results already on screen) or from asking the sources now.
pub fn play_video(
    state: &Entity<AppState>,
    item: PlayItem,
    known: Option<StreamResults>,
    cx: &mut App,
) -> Task<anyhow::Result<()>> {
    let movie = item.video_id == item.meta_id;
    let file = state
        .read(cx)
        .local_files(&item.meta_id, (!movie).then_some(item.video_id.as_str()))
        .into_iter()
        .next()
        .or_else(|| local_copy(state.read(cx), &item));
    if let Some(file) = file {
        return state.update(cx, |state, cx| state.play_local(&file, item, cx));
    }
    if item.meta_id.starts_with("local:") {
        return Task::ready(Err(anyhow!(
            "The file for this is gone. Rescan the library."
        )));
    }
    let state = state.clone();
    let lookup = match known {
        Some(results) => Task::ready(Ok(results)),
        None => state.update(cx, |s, cx| s.load_streams(&item.kind, &item.video_id, cx)),
    };
    cx.spawn(async move |cx| {
        let results = lookup.await?;
        let Some(found) = pick_stream(&results).cloned() else {
            return Err(no_stream_reason(&results));
        };
        state
            .update(cx, |state, cx| state.play_stream(&found, item, cx))
            .await
    })
}

/// A file of an unmatched title, found by its video id.
fn local_copy(state: &AppState, item: &PlayItem) -> Option<crate::library::LocalFile> {
    if !item.meta_id.starts_with("local:") {
        return None;
    }
    let title = state.library_title(&item.meta_id)?;
    title
        .videos()
        .into_iter()
        .find(|v| v.video_id == item.video_id || item.video_id == item.meta_id)
        .and_then(|v| v.versions.into_iter().next())
}

/// Plays a whole title from a poster or the Home billboard: the movie, or
/// the series' next-up episode (which needs its metadata first).
pub fn play_title(
    state: &Entity<AppState>,
    target: DetailTarget,
    cx: &mut App,
) -> Task<anyhow::Result<()>> {
    if target.is_local() {
        return play_local_title(state, &target, cx);
    }
    if target.kind != "series" {
        let item = play_item(&target, None);
        return play_video(state, item, None, cx);
    }
    let state = state.clone();
    let meta = state.update(cx, |s, cx| s.load_meta(&target.kind, &target.id, cx));
    cx.spawn(async move |cx| {
        let meta = match meta.await {
            Ok(meta) => meta,
            // Offline, a show in the library still plays from its files.
            Err(err) => {
                let local = cx.update(|cx| {
                    let has_files = !state.read(cx).local_files(&target.id, None).is_empty();
                    has_files.then(|| play_local_title(&state, &target, cx))
                });
                return match local {
                    Some(task) => task.await,
                    None => Err(err),
                };
            }
        };
        let item = state.read_with(cx, |s, _| {
            let video = next_up(&meta, &s.watch)?;
            let target = DetailTarget {
                name: meta.name.clone(),
                poster: meta.poster.clone().or(target.poster.clone()),
                ..target.clone()
            };
            Some(play_item(&target, Some(video)))
        });
        let item = item.ok_or_else(|| anyhow!("No episodes are listed for this show yet."))?;
        cx.update(|cx| play_video(&state, item, None, cx)).await
    })
}

/// An unmatched title: its first unfinished file, in episode order.
fn play_local_title(
    state: &Entity<AppState>,
    target: &DetailTarget,
    cx: &mut App,
) -> Task<anyhow::Result<()>> {
    let s = state.read(cx);
    let Some(title) = s.library_title(&target.id) else {
        return Task::ready(Err(anyhow!("This title is no longer in the library.")));
    };
    let videos = title.videos();
    let video = videos
        .iter()
        .find(|v| !s.watch_entry(&v.video_id).is_some_and(|e| e.watched))
        .or(videos.first());
    let Some(video) = video else {
        return Task::ready(Err(anyhow!("No files found for this title.")));
    };
    let Some(file) = video.versions.first().cloned() else {
        return Task::ready(Err(anyhow!("No files found for this title.")));
    };
    let item = PlayItem {
        meta_id: target.id.clone(),
        kind: target.kind.clone(),
        name: target.name.clone(),
        poster: None,
        video_id: video.video_id.clone(),
        episode_label: file.episode_label(),
    };
    state.update(cx, |state, cx| state.play_local(&file, item, cx))
}

#[cfg(test)]
mod tests {
    // Not `super::*`: that brings in gpui's `test` attribute.
    use super::next_up;
    use crate::addons::Meta;
    use crate::watch::{PlayItem, WatchState};

    fn show() -> Meta {
        serde_json::from_value(serde_json::json!({
            "id": "tt1", "type": "series", "name": "Show",
            "videos": [
                {"id": "tt1:0:1", "season": 0, "episode": 1},
                {"id": "tt1:1:1", "season": 1, "episode": 1},
                {"id": "tt1:1:2", "season": 1, "episode": 2},
                {"id": "tt1:2:1", "season": 2, "episode": 1},
            ]
        }))
        .unwrap()
    }

    fn item(video_id: &str) -> PlayItem {
        PlayItem {
            meta_id: "tt1".into(),
            kind: "series".into(),
            name: "Show".into(),
            poster: None,
            video_id: video_id.into(),
            episode_label: None,
        }
    }

    #[test]
    fn next_up_starts_at_the_first_episode() {
        let meta = show();
        assert_eq!(
            next_up(&meta, &WatchState::default()).unwrap().id,
            "tt1:1:1"
        );
    }

    #[test]
    fn next_up_resumes_or_moves_on() {
        let meta = show();
        let mut watch = WatchState::default();
        watch.record(&item("tt1:1:1"), 100.0, 3000.0);
        assert_eq!(next_up(&meta, &watch).unwrap().id, "tt1:1:1");
        watch.set_watched(&item("tt1:1:1"), true);
        assert_eq!(next_up(&meta, &watch).unwrap().id, "tt1:1:2");
        watch.set_watched(&item("tt1:2:1"), true);
        watch.entries.get_mut("tt1:2:1").unwrap().updated += 10;
        // The specials come after the last season.
        assert_eq!(next_up(&meta, &watch).unwrap().id, "tt1:0:1");
    }

    #[test]
    fn marking_an_old_episode_keeps_the_one_in_progress() {
        let meta = show();
        let mut watch = WatchState::default();
        watch.record(&item("tt1:1:2"), 600.0, 3000.0);
        watch.entries.get_mut("tt1:1:2").unwrap().updated = 1;
        watch.set_watched(&item("tt1:1:1"), true);
        assert_eq!(next_up(&meta, &watch).unwrap().id, "tt1:1:2");
    }
}
