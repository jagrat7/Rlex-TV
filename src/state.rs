//! The backend facade every view shares: settings, the local library, watch
//! history, installed sources and the torrent engine, as one GPUI entity.
//!
//! Slow work (network, disk scans, the engine) runs on the Tokio runtime;
//! methods hand back GPUI tasks so views can await results. Changes to the
//! state itself call `cx.notify()`, so views that observe it re-render.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use futures::FutureExt as _;
use futures::future::{Shared, join_all};
use gpui_kit::{AppContext as _, Context, EventEmitter, SharedString, Task};

use crate::addons::{self, Addon, CINEMETA_URL, Meta, Stream};
use crate::library::{self, Library, LibraryTitle, LocalFile};
use crate::player::{self, Launch, Progress};
use crate::runtime;
use crate::store::{self, Settings};
use crate::torrent::{self, Engine, TorrentInfo};
use crate::watch::{PlayItem, WatchEntry, WatchState};

const SETTINGS_FILE: &str = "settings.json";

/// The engine starting up; resolves to it, or to why it failed.
type EngineStart = Shared<Task<Result<Arc<Engine>, String>>>;

pub enum EngineStatus {
    /// Not needed yet: it starts when a torrent first plays, or with the app
    /// when there are torrents to resume.
    Off,
    Starting,
    Ready(Arc<Engine>),
    Failed(String),
}

/// Background problems and confirmations worth a toast.
#[derive(Clone, Debug)]
pub enum AppEvent {
    Notice(SharedString),
    Error(SharedString),
}

/// A catalog an addon offers, shown as a row of posters.
#[derive(Clone, Debug)]
pub struct CatalogRef {
    pub title: String,
    pub base_url: String,
    pub kind: String,
    pub id: String,
}

#[derive(Clone, Debug)]
pub struct CatalogRow {
    pub title: String,
    pub items: Vec<Meta>,
}

/// A playable stream and the name of the source that offered it.
#[derive(Clone, Debug)]
pub struct FoundStream {
    pub source: String,
    pub stream: Stream,
}

#[derive(Clone, Debug, Default)]
pub struct StreamResults {
    /// Deduplicated, in source order, each source's own order kept.
    pub streams: Vec<FoundStream>,
    /// Sources that failed, and why.
    pub failures: Vec<(String, String)>,
    /// How many sources were asked. Zero means none is installed for this type.
    pub asked: usize,
}

pub struct AppState {
    pub settings: Settings,
    pub library: Library,
    pub watch: WatchState,
    pub engine: EngineStatus,
    pub scanning: bool,
    client: addons::Client,
    /// The engine's startup, shared by everything waiting on it.
    engine_start: Option<EngineStart>,
}

impl EventEmitter<AppEvent> for AppState {}

impl AppState {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            settings: store::load(SETTINGS_FILE),
            library: store::load(Library::FILE),
            watch: store::load(WatchState::FILE),
            engine: EngineStatus::Off,
            scanning: false,
            client: addons::Client::new(),
            engine_start: None,
        };
        if torrent::has_saved_torrents() {
            // Resume them; the start is kept in `engine_start`.
            drop(this.engine(cx));
        }
        if !this.settings.library_folders.is_empty() {
            this.rescan(false, cx);
        }
        this
    }

    /// The torrent engine, starting it if it isn't running. After a failed
    /// start, the next call tries again.
    fn engine(&mut self, cx: &mut Context<Self>) -> EngineStart {
        if let Some(start) = &self.engine_start
            && !matches!(self.engine, EngineStatus::Failed(_))
        {
            return start.clone();
        }
        self.engine = EngineStatus::Starting;
        cx.notify();
        let start = runtime::run(Engine::start(self.settings.download_dir()));
        let task = cx
            .spawn(async move |this, cx| {
                let result = start.await.map_err(|err| format!("{err:#}"));
                this.update(cx, |this, cx| {
                    this.engine = match &result {
                        Ok(engine) => EngineStatus::Ready(engine.clone()),
                        Err(err) => {
                            log::error!("torrent engine: {err}");
                            cx.emit(AppEvent::Error(
                                format!("Torrent engine didn't start: {err}").into(),
                            ));
                            EngineStatus::Failed(err.clone())
                        }
                    };
                    cx.notify();
                })
                .ok();
                result
            })
            .shared();
        self.engine_start = Some(task.clone());
        task
    }

    // Library

    pub fn add_folder(&mut self, folder: PathBuf, cx: &mut Context<Self>) {
        if !self.settings.library_folders.contains(&folder) {
            self.settings.library_folders.push(folder);
            self.save_settings(cx);
            self.rescan(false, cx);
        }
    }

    pub fn remove_folder(&mut self, folder: &PathBuf, cx: &mut Context<Self>) {
        self.settings.library_folders.retain(|f| f != folder);
        self.save_settings(cx);
        self.rescan(false, cx);
    }

    /// Re-reads the library folders and matches new files. `retry_unmatched`
    /// also asks Cinemeta again about files it didn't know before.
    pub fn rescan(&mut self, retry_unmatched: bool, cx: &mut Context<Self>) {
        if self.scanning {
            return;
        }
        self.scanning = true;
        cx.notify();

        let folders = self.settings.library_folders.clone();
        let mut previous = self.library.clone();
        if retry_unmatched {
            previous.misses.clear();
        }
        let client = self.client.clone();
        cx.spawn(async move |this, cx| {
            let scanned = runtime::run(async move {
                let library = tokio::task::spawn_blocking(move || Library {
                    files: library::scan(&folders, &previous),
                    misses: previous.misses,
                })
                .await?;
                Ok(library)
            })
            .await;
            let library = match scanned {
                Ok(library) => library,
                Err(err) => {
                    this.update(cx, |this, cx| {
                        this.scanning = false;
                        cx.emit(AppEvent::Error(
                            format!("Library scan failed: {err:#}").into(),
                        ));
                        cx.notify();
                    })
                    .ok();
                    return;
                }
            };
            // Show the files straight away; posters follow once matched.
            this.update(cx, |this, cx| {
                this.library = library.clone();
                cx.notify();
            })
            .ok();

            let matched = runtime::run(async move {
                let mut library = library;
                library::match_files(&client, &mut library).await;
                Ok(library)
            })
            .await;
            this.update(cx, |this, cx| {
                this.scanning = false;
                if let Ok(library) = matched {
                    this.library = library;
                }
                if let Err(err) = store::save(Library::FILE, &this.library) {
                    log::error!("saving the library: {err:#}");
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    pub fn library_titles(&self) -> Vec<LibraryTitle> {
        self.library.titles()
    }

    pub fn library_title(&self, meta_id: &str) -> Option<LibraryTitle> {
        self.library.title(meta_id)
    }

    pub fn recently_added(&self, limit: usize) -> Vec<LibraryTitle> {
        self.library.recently_added(limit)
    }

    /// Local files of a title, or of one of its episodes; copies of a video
    /// best first.
    pub fn local_files(&self, meta_id: &str, video_id: Option<&str>) -> Vec<LocalFile> {
        self.library.files_for(meta_id, video_id)
    }

    // Sources

    pub fn sources(&self) -> &[Addon] {
        &self.settings.addons
    }

    /// Validates the addon by fetching its manifest, then adds it.
    pub fn install_addon(&mut self, url: &str, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        let base_url = match addons::normalize_url(url) {
            Ok(base_url) => base_url,
            Err(err) => return Task::ready(Err(err)),
        };
        if self.settings.addons.iter().any(|a| a.base_url == base_url) {
            return Task::ready(Err(anyhow!("That source is already added")));
        }
        let client = self.client.clone();
        let fetch = runtime::run(async move {
            let manifest = client.manifest(&base_url).await?;
            Ok(Addon { base_url, manifest })
        });
        cx.spawn(async move |this, cx| {
            let addon = fetch.await?;
            this.update(cx, |this, cx| {
                let name = addon.name().to_string();
                this.settings.addons.push(addon);
                this.save_settings(cx);
                cx.emit(AppEvent::Notice(format!("Added {name}").into()));
                cx.notify();
            })?;
            Ok(())
        })
    }

    pub fn remove_addon(&mut self, base_url: &str, cx: &mut Context<Self>) {
        self.settings.addons.retain(|a| a.base_url != base_url);
        self.save_settings(cx);
        cx.notify();
    }

    // Browsing

    /// Rows for the home page: Cinemeta's lists, then any catalog an installed
    /// addon offers without needing a search term.
    pub fn home_catalogs(&self) -> Vec<CatalogRef> {
        let cinemeta = [
            ("movie", "top", "Popular movies"),
            ("series", "top", "Popular series"),
            ("movie", "imdbRating", "Top rated movies"),
            ("series", "imdbRating", "Top rated series"),
        ]
        .map(|(kind, id, title)| CatalogRef {
            title: title.to_string(),
            base_url: CINEMETA_URL.to_string(),
            kind: kind.to_string(),
            id: id.to_string(),
        });
        let from_addons = self.settings.addons.iter().flat_map(|addon| {
            addon
                .manifest
                .catalogs
                .iter()
                .filter(|c| !c.needs_extra())
                .map(|c| CatalogRef {
                    title: format!("{} · {}", c.name.as_deref().unwrap_or(&c.id), addon.name()),
                    base_url: addon.base_url.clone(),
                    kind: c.kind.clone(),
                    id: c.id.clone(),
                })
        });
        cinemeta.into_iter().chain(from_addons).collect()
    }

    pub fn load_catalog(
        &self,
        catalog: &CatalogRef,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Vec<Meta>>> {
        let client = self.client.clone();
        let catalog = catalog.clone();
        cx.background_spawn(runtime::run(async move {
            client
                .catalog(&catalog.base_url, &catalog.kind, &catalog.id, &[])
                .await
        }))
    }

    /// Searches Cinemeta's movies and series, and any addon catalog that takes
    /// a search term. Rows that come back empty are left out.
    pub fn search(
        &self,
        query: &str,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Vec<CatalogRow>>> {
        let query = query.trim().to_string();
        if query.is_empty() {
            return Task::ready(Ok(Vec::new()));
        }
        let mut targets = vec![
            (
                "Movies".to_string(),
                CINEMETA_URL.to_string(),
                "movie".to_string(),
                "top".to_string(),
            ),
            (
                "Series".to_string(),
                CINEMETA_URL.to_string(),
                "series".to_string(),
                "top".to_string(),
            ),
        ];
        for addon in &self.settings.addons {
            for catalog in addon
                .manifest
                .catalogs
                .iter()
                .filter(|c| c.supports_extra("search"))
            {
                targets.push((
                    format!(
                        "{} · {}",
                        catalog.name.as_deref().unwrap_or(&catalog.id),
                        addon.name()
                    ),
                    addon.base_url.clone(),
                    catalog.kind.clone(),
                    catalog.id.clone(),
                ));
            }
        }
        let client = self.client.clone();
        cx.background_spawn(runtime::run(async move {
            let extra = [("search", query.as_str())];
            let results = join_all(
                targets
                    .iter()
                    .map(|(_, base, kind, id)| client.catalog(base, kind, id, &extra)),
            )
            .await;
            let mut rows = Vec::new();
            let mut first_error = None;
            for ((title, ..), result) in targets.into_iter().zip(results) {
                match result {
                    Ok(items) if !items.is_empty() => rows.push(CatalogRow { title, items }),
                    Ok(_) => {}
                    Err(err) => {
                        log::warn!("search in {title}: {err:#}");
                        first_error.get_or_insert(err);
                    }
                }
            }
            match first_error {
                Some(err) if rows.is_empty() => Err(err),
                _ => Ok(rows),
            }
        }))
    }

    /// Full metadata, with episodes for a series. Cinemeta answers IMDb ids;
    /// other ids go to the installed addons that claim them.
    pub fn load_meta(
        &self,
        kind: &str,
        id: &str,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<Meta>> {
        let mut providers: Vec<String> = Vec::new();
        if id.starts_with("tt") {
            providers.push(CINEMETA_URL.to_string());
        }
        providers.extend(
            self.settings
                .addons
                .iter()
                .filter(|a| a.provides("meta", kind, id))
                .map(|a| a.base_url.clone()),
        );
        let client = self.client.clone();
        let (kind, id) = (kind.to_string(), id.to_string());
        cx.background_spawn(runtime::run(async move {
            let mut last_error = anyhow!("no source has details for {id}");
            for base in providers {
                match client.meta(&base, &kind, &id).await {
                    Ok(meta) => return Ok(meta),
                    Err(err) => last_error = err,
                }
            }
            Err(last_error)
        }))
    }

    /// Asks every installed source for streams of a video.
    pub fn load_streams(
        &self,
        kind: &str,
        video_id: &str,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<StreamResults>> {
        let sources: Vec<Addon> = self
            .settings
            .addons
            .iter()
            .filter(|a| a.provides("stream", kind, video_id))
            .cloned()
            .collect();
        let client = self.client.clone();
        let (kind, video_id) = (kind.to_string(), video_id.to_string());
        cx.background_spawn(runtime::run(async move {
            let results = join_all(
                sources
                    .iter()
                    .map(|addon| client.streams(&addon.base_url, &kind, &video_id)),
            )
            .await;
            let mut out = StreamResults {
                asked: sources.len(),
                ..Default::default()
            };
            let mut seen = HashSet::new();
            for (addon, result) in sources.iter().zip(results) {
                match result {
                    Ok(streams) => {
                        for stream in streams {
                            if seen.insert(stream.dedupe_key()) {
                                out.streams.push(FoundStream {
                                    source: addon.name().to_string(),
                                    stream,
                                });
                            }
                        }
                    }
                    Err(err) => out
                        .failures
                        .push((addon.name().to_string(), format!("{err:#}"))),
                }
            }
            Ok(out)
        }))
    }

    // Playback

    pub fn play_local(
        &mut self,
        file: &LocalFile,
        item: PlayItem,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        self.launch(file.path.to_string_lossy().into_owned(), item, cx)
    }

    /// Plays a stream from a source: directly for URL streams, through the
    /// torrent engine for torrents.
    pub fn play_stream(
        &mut self,
        found: &FoundStream,
        item: PlayItem,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let stream = found.stream.clone();
        let Some(info_hash) = stream.info_hash.clone() else {
            return match stream.url {
                Some(url) => self.launch(url, item, cx),
                None => Task::ready(Err(anyhow!("This stream has nothing to play"))),
            };
        };
        let engine = self.engine(cx);
        cx.spawn(async move |this, cx| {
            let engine = engine
                .await
                .map_err(|err| anyhow!("The torrent engine isn't running: {err}"))?;
            let url = runtime::run(async move {
                engine
                    .stream(
                        &info_hash,
                        stream.file_idx.map(|i| i as usize),
                        stream.behavior_hints.filename.as_deref(),
                        &stream.trackers(),
                    )
                    .await
            })
            .await?;
            this.update(cx, |this, cx| this.launch(url, item, cx))?
                .await
        })
    }

    /// Opens `target` in the default player, resuming where the history says,
    /// and records progress as the player reports it. The task resolves once
    /// the player has started.
    fn launch(
        &mut self,
        target: String,
        item: PlayItem,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        let start = self.watch.resume_position(&item.video_id);
        self.watch.started(&item);
        self.save_watch();
        cx.notify();

        let title = match &item.episode_label {
            Some(label) => format!("{} — {label}", item.name),
            None => item.name.clone(),
        };
        let started = runtime::run(player::play(Launch {
            target,
            title,
            start,
        }));
        cx.spawn(async move |this, cx| {
            let mut progress = started.await?;
            this.update(cx, |_, cx| {
                cx.spawn(async move |this, cx| {
                    while let Some(Progress::Position { position, duration }) =
                        progress.recv().await
                    {
                        let alive = this.update(cx, |this, cx| {
                            this.watch.record(&item, position, duration);
                            this.save_watch();
                            cx.notify();
                        });
                        if alive.is_err() {
                            break;
                        }
                    }
                })
                .detach();
            })?;
            Ok(())
        })
    }

    pub fn set_watched(&mut self, item: &PlayItem, watched: bool, cx: &mut Context<Self>) {
        self.watch.set_watched(item, watched);
        self.save_watch();
        cx.notify();
    }

    pub fn watch_entry(&self, video_id: &str) -> Option<&WatchEntry> {
        self.watch.get(video_id)
    }

    pub fn continue_watching(&self) -> Vec<WatchEntry> {
        self.watch.continue_watching()
    }

    // Downloads

    /// Empty unless the engine is running.
    pub fn torrents(&self) -> Vec<TorrentInfo> {
        match &self.engine {
            EngineStatus::Ready(engine) => engine.torrents(),
            _ => Vec::new(),
        }
    }

    pub fn pause_torrent(&self, id: usize, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        self.with_engine(cx, move |engine| async move { engine.pause(id).await })
    }

    pub fn resume_torrent(&self, id: usize, cx: &mut Context<Self>) -> Task<anyhow::Result<()>> {
        self.with_engine(cx, move |engine| async move { engine.resume(id).await })
    }

    pub fn remove_torrent(
        &self,
        id: usize,
        delete_files: bool,
        cx: &mut Context<Self>,
    ) -> Task<anyhow::Result<()>> {
        self.with_engine(cx, move |engine| async move {
            engine.remove(id, delete_files).await
        })
    }

    fn with_engine<F, Fut>(&self, cx: &mut Context<Self>, f: F) -> Task<anyhow::Result<()>>
    where
        F: FnOnce(Arc<Engine>) -> Fut,
        Fut: Future<Output = anyhow::Result<()>> + Send + 'static,
    {
        match &self.engine {
            EngineStatus::Ready(engine) => cx.background_spawn(runtime::run(f(engine.clone()))),
            _ => Task::ready(Err(anyhow!("The torrent engine isn't running"))),
        }
    }

    // Persistence

    fn save_settings(&self, cx: &mut Context<Self>) {
        if let Err(err) = store::save(SETTINGS_FILE, &self.settings) {
            cx.emit(AppEvent::Error(
                format!("Couldn't save settings: {err:#}").into(),
            ));
        }
    }

    fn save_watch(&self) {
        if let Err(err) = store::save(WatchState::FILE, &self.watch) {
            log::error!("saving watch history: {err:#}");
        }
    }
}
