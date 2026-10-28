//! Watch history: what was played, how far in, and what's finished.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::store::now;

/// Past this fraction of the runtime a video counts as watched.
const WATCHED_AT: f64 = 0.9;
/// Positions this close to either end aren't worth resuming from.
const RESUME_MARGIN_SECS: f64 = 30.0;

/// Identifies what is being played, for history and resume.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayItem {
    /// The title: an IMDb id like `tt0903747`, an addon's own id, or
    /// `local:<path>` for a file that matched nothing.
    pub meta_id: String,
    /// Stremio type: `movie` or `series`.
    pub kind: String,
    pub name: String,
    pub poster: Option<String>,
    /// `tt0903747:1:2` for an episode; the same as `meta_id` for a movie.
    pub video_id: String,
    /// Shown under the title, e.g. `S1 E2 · Pilot`.
    pub episode_label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WatchEntry {
    pub meta_id: String,
    pub kind: String,
    pub name: String,
    pub poster: Option<String>,
    pub video_id: String,
    pub episode_label: Option<String>,
    /// Seconds.
    pub position: f64,
    /// Seconds; 0 when the player never reported one.
    pub duration: f64,
    pub watched: bool,
    /// Unix seconds of the last change.
    pub updated: u64,
}

impl WatchEntry {
    fn new(item: &PlayItem) -> Self {
        Self {
            meta_id: item.meta_id.clone(),
            kind: item.kind.clone(),
            name: item.name.clone(),
            poster: item.poster.clone(),
            video_id: item.video_id.clone(),
            episode_label: item.episode_label.clone(),
            position: 0.0,
            duration: 0.0,
            watched: false,
            updated: now(),
        }
    }

    /// 0.0–1.0, for progress bars.
    pub fn progress(&self) -> f32 {
        if self.watched {
            1.0
        } else if self.duration > 0.0 {
            (self.position / self.duration).clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }

    pub fn resume_position(&self) -> Option<f64> {
        let resumable = !self.watched
            && self.position > RESUME_MARGIN_SECS
            && (self.duration <= 0.0 || self.position < self.duration - RESUME_MARGIN_SECS);
        resumable.then_some(self.position)
    }
}

/// Keyed by video id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchState {
    pub entries: HashMap<String, WatchEntry>,
}

impl WatchState {
    pub const FILE: &str = "watch.json";

    pub fn get(&self, video_id: &str) -> Option<&WatchEntry> {
        self.entries.get(video_id)
    }

    pub fn resume_position(&self, video_id: &str) -> Option<f64> {
        self.get(video_id).and_then(WatchEntry::resume_position)
    }

    /// Notes that playback of `item` started, so it shows up as recently played
    /// even if the player never reports a position.
    pub fn started(&mut self, item: &PlayItem) {
        let entry = self
            .entries
            .entry(item.video_id.clone())
            .or_insert_with(|| WatchEntry::new(item));
        entry.updated = now();
        // Rewatching something finished starts it over.
        if entry.watched {
            entry.watched = false;
            entry.position = 0.0;
        }
    }

    /// Records a position reported by the player.
    pub fn record(&mut self, item: &PlayItem, position: f64, duration: f64) {
        let entry = self
            .entries
            .entry(item.video_id.clone())
            .or_insert_with(|| WatchEntry::new(item));
        entry.position = position.max(0.0);
        if duration > 0.0 {
            entry.duration = duration;
        }
        if entry.duration > 0.0 && entry.position / entry.duration >= WATCHED_AT {
            entry.watched = true;
        }
        entry.updated = now();
    }

    pub fn set_watched(&mut self, item: &PlayItem, watched: bool) {
        let entry = self
            .entries
            .entry(item.video_id.clone())
            .or_insert_with(|| WatchEntry::new(item));
        entry.watched = watched;
        if !watched {
            entry.position = 0.0;
        }
        entry.updated = now();
    }

    /// The most recent unfinished video of each title, newest first.
    pub fn continue_watching(&self) -> Vec<WatchEntry> {
        let mut latest: HashMap<&str, &WatchEntry> = HashMap::new();
        for entry in self.entries.values() {
            let newer = latest
                .get(entry.meta_id.as_str())
                .is_none_or(|seen| entry.updated > seen.updated);
            if newer {
                latest.insert(&entry.meta_id, entry);
            }
        }
        let mut list: Vec<WatchEntry> = latest
            .into_values()
            .filter(|e| !e.watched)
            .cloned()
            .collect();
        list.sort_by_key(|e| std::cmp::Reverse(e.updated));
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn episode(n: u32) -> PlayItem {
        PlayItem {
            meta_id: "tt0903747".into(),
            kind: "series".into(),
            name: "Breaking Bad".into(),
            poster: None,
            video_id: format!("tt0903747:1:{n}"),
            episode_label: Some(format!("S1 E{n}")),
        }
    }

    #[test]
    fn marks_watched_near_the_end() {
        let mut state = WatchState::default();
        state.record(&episode(1), 600.0, 3000.0);
        assert!(!state.get("tt0903747:1:1").unwrap().watched);
        assert_eq!(state.resume_position("tt0903747:1:1"), Some(600.0));
        state.record(&episode(1), 2750.0, 3000.0);
        assert!(state.get("tt0903747:1:1").unwrap().watched);
        assert_eq!(state.resume_position("tt0903747:1:1"), None);
    }

    #[test]
    fn continue_watching_keeps_latest_unfinished_per_title() {
        let mut state = WatchState::default();
        state.record(&episode(1), 2900.0, 3000.0);
        state.entries.get_mut("tt0903747:1:1").unwrap().updated = 1;
        state.record(&episode(2), 120.0, 3000.0);
        let list = state.continue_watching();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].video_id, "tt0903747:1:2");
    }

    #[test]
    fn replaying_a_finished_video_starts_over() {
        let mut state = WatchState::default();
        state.set_watched(&episode(3), true);
        state.started(&episode(3));
        assert!(!state.get("tt0903747:1:3").unwrap().watched);
    }
}
