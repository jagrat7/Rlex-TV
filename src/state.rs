//! The backend every page shares: settings, the local library, watch
//! history, installed sources, review scores and the torrent engine.
//!
//! Nothing here knows about the UI. Slow work is handed back as `Send`
//! futures for the UI to run on Tokio ([`crate::ui::Bg`]); what they return
//! comes back through the plain methods that apply it.

use std::collections::HashSet;
use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::anyhow;
use futures::FutureExt as _;
use futures::future::{BoxFuture, Shared, join_all};

use crate::addons::{self, Addon, CINEMETA_URL, Meta, Stream};
use crate::library::{Library, LibraryTitle, LocalFile};
use crate::ratings::{Fetched, OmdbError, Ratings, RatingsCache, Sources};
use crate::store::{self, Settings};
use crate::torrent::{Engine, TorrentInfo};
use crate::watch::WatchState;

const SETTINGS_FILE: &str = "settings.json";

/// The engine starting up, shared by everything waiting on it.
pub type EngineStart = Shared<BoxFuture<'static, Result<Arc<Engine>, String>>>;

pub enum EngineStatus {
    /// Not needed yet: it starts when a torrent first plays, or with the app
    /// when there are torrents to resume.
    Off,
    Starting,
    Ready(Arc<Engine>),
    Failed(String),
}

/// A catalog an addon offers, shown as a row of posters.
#[derive(Clone, Debug, PartialEq)]
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

impl StreamResults {
    pub fn playable(&self) -> impl Iterator<Item = &FoundStream> {
        self.streams.iter().filter(|f| f.stream.is_playable())
    }
}

pub struct Core {
    pub settings: Settings,
    pub library: Library,
    pub watch: WatchState,
    pub engine: EngineStatus,
    pub scanning: bool,
    pub client: addons::Client,
    engine_start: Option<EngineStart>,
    ratings: RatingsState,
}

impl Core {
    /// Everything saved last time.
    pub fn load() -> Self {
        Self {
            settings: store::load(SETTINGS_FILE),
            library: store::load(Library::FILE),
            watch: store::load(WatchState::FILE),
            engine: EngineStatus::Off,
            scanning: false,
            client: addons::Client::new(),
            engine_start: None,
            ratings: RatingsState::new(),
        }
    }

    /// Nothing saved: no folders, sources, history or scores, for tests.
    #[cfg(test)]
    pub fn empty() -> Self {
        Self {
            settings: Settings::default(),
            library: Library::default(),
            watch: WatchState::default(),
            engine: EngineStatus::Off,
            scanning: false,
            client: addons::Client::new(),
            engine_start: None,
            ratings: RatingsState {
                cache: RatingsCache::default(),
                sources: Sources::new(),
                in_flight: HashSet::new(),
                key_error: None,
            },
        }
    }

    /// The torrent engine's start, beginning it if it isn't running; `true`
    /// with a start that just began, for the caller to follow. After a
    /// failed start, the next call tries again.
    pub fn start_engine(&mut self) -> (EngineStart, bool) {
        if let Some(start) = &self.engine_start
            && !matches!(self.engine, EngineStatus::Failed(_))
        {
            return (start.clone(), false);
        }
        self.engine = EngineStatus::Starting;
        let dir = self.settings.download_dir();
        let start = async move {
            // Spawned so the session lives on the runtime, whoever polls this.
            crate::runtime::get()
                .spawn(Engine::start(dir))
                .await
                .map_err(|err| err.to_string())
                .and_then(|result| result.map_err(|err| format!("{err:#}")))
        }
        .boxed()
        .shared();
        self.engine_start = Some(start.clone());
        (start, true)
    }

    pub fn engine_started(&mut self, result: &Result<Arc<Engine>, String>) {
        self.engine = match result {
            Ok(engine) => EngineStatus::Ready(engine.clone()),
            Err(err) => EngineStatus::Failed(err.clone()),
        };
    }

    // Library

    /// Adds a folder; `false` when it was already there.
    pub fn add_folder(&mut self, folder: PathBuf) -> bool {
        if self.settings.library_folders.contains(&folder) {
            return false;
        }
        self.settings.library_folders.push(folder);
        true
    }

    pub fn remove_folder(&mut self, folder: &PathBuf) {
        self.settings.library_folders.retain(|f| f != folder);
    }

    /// What a rescan starts from, marking one as running; `None` while one
    /// already is. `retry_unmatched` asks Cinemeta again about files it
    /// didn't know before.
    pub fn begin_scan(&mut self, retry_unmatched: bool) -> Option<(Vec<PathBuf>, Library)> {
        if self.scanning {
            return None;
        }
        self.scanning = true;
        let mut previous = self.library.clone();
        if retry_unmatched {
            previous.misses.clear();
        }
        Some((self.settings.library_folders.clone(), previous))
    }

    pub fn library_title(&self, meta_id: &str) -> Option<LibraryTitle> {
        self.library.title(meta_id)
    }

    /// Local files of a title, or of one of its episodes; copies of a video
    /// best first.
    pub fn local_files(&self, meta_id: &str, video_id: Option<&str>) -> Vec<LocalFile> {
        self.library.files_for(meta_id, video_id)
    }

    pub fn save_library(&self) {
        if let Err(err) = store::save(Library::FILE, &self.library) {
            log::error!("saving the library: {err:#}");
        }
    }

    // Sources

    pub fn sources(&self) -> &[Addon] {
        &self.settings.addons
    }

    /// Fetches an addon's manifest, to validate it before it's added.
    pub fn fetch_addon(
        &self,
        url: &str,
    ) -> anyhow::Result<impl Future<Output = anyhow::Result<Addon>> + Send + 'static> {
        let base_url = addons::normalize_url(url)?;
        if self.settings.addons.iter().any(|a| a.base_url == base_url) {
            return Err(anyhow!("That source is already added"));
        }
        let client = self.client.clone();
        Ok(async move {
            let manifest = client.manifest(&base_url).await?;
            Ok(Addon { base_url, manifest })
        })
    }

    pub fn add_addon(&mut self, addon: Addon) -> anyhow::Result<()> {
        self.settings.addons.push(addon);
        self.save_settings()
    }

    pub fn remove_addon(&mut self, base_url: &str) -> anyhow::Result<()> {
        self.settings.addons.retain(|a| a.base_url != base_url);
        self.save_settings()
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

    pub fn catalog(
        &self,
        catalog: &CatalogRef,
    ) -> impl Future<Output = anyhow::Result<Vec<Meta>>> + Send + 'static {
        let client = self.client.clone();
        let catalog = catalog.clone();
        async move {
            client
                .catalog(&catalog.base_url, &catalog.kind, &catalog.id, &[])
                .await
        }
    }

    /// Cinemeta's popular titles of one kind in one genre, for "More like
    /// this".
    pub fn genre(
        &self,
        kind: &str,
        genre: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<Meta>>> + Send + 'static {
        let client = self.client.clone();
        let (kind, genre) = (kind.to_string(), genre.to_string());
        async move {
            client
                .catalog(CINEMETA_URL, &kind, "top", &[("genre", &genre)])
                .await
        }
    }

    /// Searches Cinemeta's movies and series, and any addon catalog that takes
    /// a search term. Rows that come back empty are left out.
    pub fn search(
        &self,
        query: &str,
    ) -> impl Future<Output = anyhow::Result<Vec<CatalogRow>>> + Send + 'static {
        let query = query.trim().to_string();
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
        async move {
            if query.is_empty() {
                return Ok(Vec::new());
            }
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
        }
    }

    /// Full metadata, with episodes for a series. Cinemeta answers IMDb ids;
    /// other ids go to the installed addons that claim them.
    pub fn meta(
        &self,
        kind: &str,
        id: &str,
    ) -> impl Future<Output = anyhow::Result<Meta>> + Send + 'static {
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
        async move {
            let mut last_error = anyhow!("no source has details for {id}");
            for base in providers {
                match client.meta(&base, &kind, &id).await {
                    Ok(meta) => return Ok(meta),
                    Err(err) => last_error = err,
                }
            }
            Err(last_error)
        }
    }

    /// Asks every installed source for streams of a video.
    pub fn streams(
        &self,
        kind: &str,
        video_id: &str,
    ) -> impl Future<Output = anyhow::Result<StreamResults>> + Send + 'static {
        let sources: Vec<Addon> = self
            .settings
            .addons
            .iter()
            .filter(|a| a.provides("stream", kind, video_id))
            .cloned()
            .collect();
        let client = self.client.clone();
        let (kind, video_id) = (kind.to_string(), video_id.to_string());
        async move {
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
        }
    }

    // Downloads

    /// Empty unless the engine is running.
    pub fn torrents(&self) -> Vec<TorrentInfo> {
        match &self.engine {
            EngineStatus::Ready(engine) => engine.torrents(),
            _ => Vec::new(),
        }
    }

    pub fn ready_engine(&self) -> anyhow::Result<Arc<Engine>> {
        match &self.engine {
            EngineStatus::Ready(engine) => Ok(engine.clone()),
            _ => Err(anyhow!("The torrent engine isn't running")),
        }
    }

    // Persistence

    pub fn save_settings(&self) -> anyhow::Result<()> {
        store::save(SETTINGS_FILE, &self.settings)
            .map_err(|err| anyhow!("Couldn't save settings: {err:#}"))
    }

    pub fn save_watch(&self) {
        if let Err(err) = store::save(WatchState::FILE, &self.watch) {
            log::error!("saving watch history: {err:#}");
        }
    }
}

// Ratings

/// Review scores: the disk cache, fetches in flight, and any problem with
/// the OMDb key.
struct RatingsState {
    cache: RatingsCache,
    sources: Sources,
    in_flight: HashSet<String>,
    /// The OMDb key itself failed (invalid, or out of quota). Only Wikidata
    /// is asked until the key changes or the problem expires, so a bad key
    /// doesn't burn requests.
    key_error: Option<KeyError>,
}

struct KeyError {
    message: String,
    /// Unix seconds when the key is tried again. A shared built-in key gets
    /// its daily quota back, so a failure isn't forever.
    retry_at: u64,
}

impl KeyError {
    fn active(&self) -> bool {
        store::now() < self.retry_at
    }
}

impl RatingsState {
    fn new() -> Self {
        Self {
            cache: RatingsCache::load(),
            sources: Sources::new(),
            in_flight: HashSet::new(),
            key_error: None,
        }
    }
}

/// A ratings fetch to run: the key it uses, and the request.
pub type RatingsRequest = (Option<String>, BoxFuture<'static, anyhow::Result<Fetched>>);

impl Core {
    /// Cached scores for an IMDb id, possibly older than a week.
    pub fn ratings(&self, imdb_id: &str) -> Option<Ratings> {
        self.ratings.cache.get(imdb_id).map(|e| e.ratings.clone())
    }

    /// The request that refreshes a title's scores: Wikidata first, then
    /// OMDb for what it lacks. `None` when the cache is fresh, the id isn't
    /// an IMDb id, or a fetch for it is already running.
    ///
    /// Ask only for the title on screen, never for catalog rows: the free
    /// OMDb key allows 1000 requests a day.
    pub fn ratings_request(&mut self, imdb_id: &str) -> Option<RatingsRequest> {
        if !crate::ratings::is_imdb_id(imdb_id) {
            return None;
        }
        if self
            .ratings
            .cache
            .get(imdb_id)
            .is_some_and(|e| e.is_fresh(store::now()))
        {
            return None;
        }
        if !self.ratings.in_flight.insert(imdb_id.to_string()) {
            return None;
        }
        let key = self.usable_omdb_key();
        let sources = self.ratings.sources.clone();
        let id = imdb_id.to_string();
        let request = {
            let key = key.clone();
            async move { sources.fetch(key.as_deref(), &id).await }.boxed()
        };
        Some((key, request))
    }

    pub fn ratings_fetched(
        &mut self,
        imdb_id: &str,
        key: Option<String>,
        result: anyhow::Result<Fetched>,
    ) {
        self.ratings.in_flight.remove(imdb_id);
        let same_key = self.usable_omdb_key() == key;
        // A key exists but was held back because it had failed.
        let held_back = key.is_none() && self.omdb_key().is_some();
        let (ratings, omdb_error) = match result {
            Ok(Fetched {
                ratings,
                omdb_error,
            }) => (Some(ratings), omdb_error),
            Err(err) => (None, Some(err)),
        };
        // Whatever went wrong with OMDb, keep what Wikidata had until it answers.
        let mut retry_soon = omdb_error.is_some();
        if let Some(err) = &omdb_error {
            let key_problem = err
                .downcast_ref::<OmdbError>()
                .is_some_and(OmdbError::is_key_error);
            if key_problem && same_key {
                let message = format!("{err:#}");
                log::warn!("ratings: {message}");
                self.ratings.key_error = Some(KeyError {
                    message,
                    retry_at: store::now() + crate::ratings::RETRY_SECS,
                });
            }
        }
        // Scores fetched for an older key may lack what the new one adds;
        // the next view fetches again.
        let Some(ratings) = ratings.filter(|_| same_key) else {
            return;
        };
        // Without a key that is only resting, OMDb may add more soon.
        retry_soon |= !ratings.has_critics() && held_back;
        if retry_soon {
            self.ratings.cache.insert_briefly(imdb_id, ratings);
        } else {
            self.ratings.cache.insert(imdb_id, ratings);
        }
        self.ratings.cache.save();
    }

    /// Saves the user's own OMDb key (`None` or blank removes it, going back
    /// to the built-in key), forgets any problem the old key had, and drops
    /// cached titles the new key might fill in.
    pub fn set_omdb_key(&mut self, key: Option<String>) -> anyhow::Result<()> {
        let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        self.settings.omdb_api_key = key;
        self.ratings.key_error = None;
        self.ratings.in_flight.clear();
        self.ratings.cache.drop_incomplete();
        self.ratings.cache.save();
        self.save_settings()
    }

    /// Whether the user saved a key of their own.
    pub fn has_own_omdb_key(&self) -> bool {
        self.own_omdb_key().is_some()
    }

    /// Why the OMDb key isn't working, if it isn't.
    pub fn ratings_error(&self) -> Option<&str> {
        let error = self.ratings.key_error.as_ref().filter(|e| e.active())?;
        Some(error.message.as_str())
    }

    /// One OMDb request with `key` (blank: the key in use), caching nothing.
    pub fn omdb_test(
        &self,
        key: &str,
    ) -> anyhow::Result<(
        String,
        impl Future<Output = anyhow::Result<Ratings>> + Send + 'static,
    )> {
        let key = match key.trim() {
            "" => self.omdb_key(),
            typed => Some(typed.to_string()),
        }
        .ok_or_else(|| anyhow!("Enter a key first"))?;
        let sources = self.ratings.sources.clone();
        let request = {
            let key = key.clone();
            async move { sources.test_omdb(&key).await }
        };
        Ok((key, request))
    }

    /// A test of `key` passed: if it's the key in use, its error is over.
    pub fn omdb_key_works(&mut self, key: &str) {
        if self.omdb_key().as_deref() == Some(key) {
            self.ratings.key_error = None;
        }
    }

    fn own_omdb_key(&self) -> Option<String> {
        self.settings
            .omdb_api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
    }

    /// The user's key, else the one built in.
    fn omdb_key(&self) -> Option<String> {
        self.own_omdb_key()
            .or_else(|| crate::ratings::built_in_key().map(str::to_string))
    }

    /// [`Self::omdb_key`], unless it failed within the last hour.
    fn usable_omdb_key(&self) -> Option<String> {
        let resting = self
            .ratings
            .key_error
            .as_ref()
            .is_some_and(KeyError::active);
        self.omdb_key().filter(|_| !resting)
    }
}
