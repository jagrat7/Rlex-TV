//! Playback: start a title or episode without choosing a source, or a
//! chosen file or stream, in the desktop's default player.
//!
//! The pick is the best copy on this computer, then the first direct (URL)
//! stream a source offers, then the first torrent. A series without a chosen
//! episode plays its next-up episode: the one in progress, the one after the
//! last finished, or the first.

use anyhow::anyhow;
use futures::FutureExt as _;
use tokio::sync::mpsc::UnboundedReceiver;

use super::{App, Bg, DetailTarget};
use crate::addons::{Meta, Video};
use crate::library::{LocalFile, LocalVideo};
use crate::player::{self, Launch, Progress};
use crate::state::{EngineStart, FoundStream, StreamResults};
use crate::watch::{PlayItem, WatchEntry, WatchState};

impl App {
    /// Plays a whole title from a poster or the Home billboard: the movie,
    /// or the series' next-up episode (which needs its metadata first).
    pub fn play_title(&mut self, target: DetailTarget) {
        if !self.claim(&target.id) {
            return;
        }
        if target.is_local() {
            return self.play_local_title(&target);
        }
        if target.kind != "series" {
            return self.play_claimed(play_item(&target, None), None);
        }
        let meta = self.core.meta(&target.kind, &target.id);
        self.bg.run(meta, move |s, meta, _| match meta {
            Ok(meta) => match next_up(&meta, &s.core.watch) {
                Some(video) => {
                    let target = DetailTarget {
                        name: meta.name.clone(),
                        poster: meta.poster.clone().or(target.poster),
                        ..target
                    };
                    s.play_claimed(play_item(&target, Some(video)), None);
                }
                None => s.play_failed(anyhow!("No episodes are listed for this show yet.")),
            },
            // Offline, a show in the library still plays from its files.
            Err(_) if !s.core.local_files(&target.id, None).is_empty() => {
                s.play_local_title(&target)
            }
            Err(err) => s.play_failed(err),
        });
    }

    /// Plays one video: a local copy if there is one, otherwise a stream from
    /// `known` (results already on screen) or from asking the sources now.
    pub fn play_video(&mut self, item: PlayItem, known: Option<StreamResults>) {
        if self.claim(&item.video_id) {
            self.play_claimed(item, known);
        }
    }

    pub fn play_file(&mut self, file: LocalFile, item: PlayItem) {
        let path = file.path.to_string_lossy().into_owned();
        if self.claim(&path) {
            self.launch(path, item);
        }
    }

    pub fn play_stream(&mut self, found: FoundStream, item: PlayItem) {
        if self.claim(&found.stream.dedupe_key()) {
            self.stream(found, item);
        }
    }

    pub fn set_watched(&mut self, item: &PlayItem, watched: bool) {
        self.core.watch.set_watched(item, watched);
        self.core.save_watch();
    }

    /// Whether `key` is starting, for spinners on what was pressed.
    pub fn is_starting(&self, key: &str) -> bool {
        self.starting.as_deref() == Some(key)
    }

    /// Marks `key` as starting, unless another player still is.
    fn claim(&mut self, key: &str) -> bool {
        if self.starting.is_some() {
            self.toast_info("Still starting the last one. Torrents can take a few seconds.");
            return false;
        }
        self.starting = Some(key.to_string());
        true
    }

    fn play_failed(&mut self, err: anyhow::Error) {
        self.starting = None;
        self.toast_error(format!("Couldn't play: {err:#}"));
    }

    fn play_claimed(&mut self, item: PlayItem, known: Option<StreamResults>) {
        let movie = item.video_id == item.meta_id;
        let file = self
            .core
            .local_files(&item.meta_id, (!movie).then_some(item.video_id.as_str()))
            .into_iter()
            .next()
            .or_else(|| self.local_copy(&item));
        if let Some(file) = file {
            return self.launch(file.path.to_string_lossy().into_owned(), item);
        }
        if item.meta_id.starts_with("local:") {
            return self.play_failed(anyhow!("The file for this is gone. Rescan the library."));
        }
        let lookup = match known {
            Some(results) => futures::future::ready(Ok(results)).left_future(),
            None => self.core.streams(&item.kind, &item.video_id).right_future(),
        };
        self.bg.run(lookup, move |s, results, _| {
            let picked =
                results.and_then(|r| pick_stream(&r).cloned().ok_or_else(|| no_stream_reason(&r)));
            match picked {
                Ok(found) => s.stream(found, item),
                Err(err) => s.play_failed(err),
            }
        });
    }

    /// A claimed stream: played directly when it's a URL, through the
    /// torrent engine when it's a torrent.
    fn stream(&mut self, found: FoundStream, item: PlayItem) {
        let stream = found.stream;
        let Some(info_hash) = stream.info_hash.clone() else {
            return match stream.url {
                Some(url) => self.launch(url, item),
                None => self.play_failed(anyhow!("This stream has nothing to play")),
            };
        };
        let engine = self.start_engine();
        let url = async move {
            let engine = engine
                .await
                .map_err(|err| anyhow!("The torrent engine isn't running: {err}"))?;
            let file = stream.file_idx.map(|i| i as usize);
            engine
                .stream(
                    &info_hash,
                    file,
                    stream.behavior_hints.filename.as_deref(),
                    &stream.trackers(),
                )
                .await
        };
        self.bg.run(url, move |s, url, _| match url {
            Ok(url) => s.launch(url, item),
            Err(err) => s.play_failed(err),
        });
    }

    /// The torrent engine, starting it if it isn't running.
    pub fn start_engine(&mut self) -> EngineStart {
        let (start, began) = self.core.start_engine();
        if began {
            self.bg.run(start.clone(), |s, result, _| {
                if let Err(err) = &result {
                    log::error!("torrent engine: {err}");
                    s.toast_error(format!("Torrent engine didn't start: {err}"));
                }
                s.core.engine_started(&result);
            });
        }
        start
    }

    /// Opens `target` in the default player, resuming where the history
    /// says, and records progress as the player reports it.
    fn launch(&mut self, target: String, item: PlayItem) {
        let start = self.core.watch.resume_position(&item.video_id);
        self.core.watch.started(&item);
        self.core.save_watch();
        let title = match &item.episode_label {
            Some(label) => format!("{} — {label}", item.name),
            None => item.name.clone(),
        };
        let bg = self.bg.clone();
        self.bg.run(
            player::play(Launch {
                target,
                title,
                start,
            }),
            move |s, started, _| {
                s.starting = None;
                match started {
                    Ok(progress) => follow(bg, item, progress),
                    Err(err) => s.toast_error(format!("Couldn't play: {err:#}")),
                }
            },
        );
    }

    /// A file of an unmatched title, found by its video id.
    fn local_copy(&self, item: &PlayItem) -> Option<LocalFile> {
        if !item.meta_id.starts_with("local:") {
            return None;
        }
        let title = self.core.library_title(&item.meta_id)?;
        title
            .videos()
            .into_iter()
            .find(|v| v.video_id == item.video_id || item.video_id == item.meta_id)
            .and_then(|v| v.versions.into_iter().next())
    }

    /// An unmatched title: the file in progress, else its first unfinished
    /// file, in episode order.
    fn play_local_title(&mut self, target: &DetailTarget) {
        let Some(title) = self.core.library_title(&target.id) else {
            return self.play_failed(anyhow!("This title is no longer in the library."));
        };
        let videos = title.videos();
        let watch = &self.core.watch;
        let entry = |v: &LocalVideo| watch.get(&v.video_id);
        let in_progress = videos
            .iter()
            .filter_map(|v| {
                entry(v)
                    .filter(|e| !e.watched && e.position > 0.0)
                    .map(|e| (v, e))
            })
            .max_by_key(|(_, e)| e.updated)
            .map(|(v, _)| v);
        let video = in_progress
            .or_else(|| videos.iter().find(|v| !entry(v).is_some_and(|e| e.watched)))
            .or(videos.first());
        let Some((video, file)) = video.and_then(|v| Some((v, v.versions.first().cloned()?)))
        else {
            return self.play_failed(anyhow!("No files found for this title."));
        };
        let item = local_item(target, video);
        self.launch(file.path.to_string_lossy().into_owned(), item);
    }
}

/// Records what the player reports until it closes.
fn follow(bg: Bg, item: PlayItem, mut progress: UnboundedReceiver<Progress>) {
    crate::runtime::get().spawn(async move {
        while let Some(Progress::Position { position, duration }) = progress.recv().await {
            let item = item.clone();
            bg.update(move |s, _| {
                s.core.watch.record(&item, position, duration);
                s.core.save_watch();
            });
        }
    });
}

/// The first stream worth playing without asking: direct links start
/// faster than torrents, so they win.
pub fn pick_stream(results: &StreamResults) -> Option<&FoundStream> {
    results
        .playable()
        .find(|f| !f.stream.is_torrent())
        .or_else(|| results.playable().next())
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
/// one in progress, else the first unfinished one after the latest finished,
/// else the first. In-progress wins, and finished episodes are skipped, so
/// ticking off an old episode doesn't move the pick back.
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
    let Some(index) = latest(true) else {
        return episodes.first().copied();
    };
    let watched = |v: &&&Video| watch.get(&v.id).is_some_and(|e| e.watched);
    let after = &episodes[index + 1..];
    after
        .iter()
        .find(|v| !watched(v))
        // All of the rest seen: a rewatch, so go on in order.
        .or(after.first())
        // Past the last episode, start the show again.
        .or(episodes.first())
        .copied()
}

/// What a Play button for `video_id` says: Resume at a position, or Play.
pub fn play_label(watch: &WatchState, video_id: &str, episode: Option<&str>) -> String {
    let resume = watch.get(video_id).and_then(WatchEntry::resume_position);
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

/// The history entry for a file of a title that matched nothing.
pub fn local_item(target: &DetailTarget, video: &LocalVideo) -> PlayItem {
    PlayItem {
        meta_id: target.id.clone(),
        kind: target.kind.clone(),
        name: target.name.clone(),
        poster: None,
        video_id: video.video_id.clone(),
        episode_label: video.versions.first().and_then(LocalFile::episode_label),
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

#[cfg(test)]
mod tests {
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

    #[test]
    fn remarking_an_old_episode_skips_ones_already_seen() {
        let meta = show();
        let mut watch = WatchState::default();
        for (n, id) in ["tt1:1:1", "tt1:1:2"].into_iter().enumerate() {
            watch.set_watched(&item(id), true);
            watch.entries.get_mut(id).unwrap().updated = n as u64;
        }
        // S1 E1 ticked again last: S1 E2 is done, so S2 E1 is next.
        watch.entries.get_mut("tt1:1:1").unwrap().updated = 10;
        assert_eq!(next_up(&meta, &watch).unwrap().id, "tt1:2:1");
    }

    #[test]
    fn a_rewatch_goes_on_in_order() {
        let meta = show();
        let mut watch = WatchState::default();
        for id in ["tt1:1:1", "tt1:1:2", "tt1:2:1", "tt1:0:1"] {
            watch.set_watched(&item(id), true);
            watch.entries.get_mut(id).unwrap().updated = 1;
        }
        watch.entries.get_mut("tt1:1:1").unwrap().updated = 10;
        assert_eq!(next_up(&meta, &watch).unwrap().id, "tt1:1:2");
    }
}
