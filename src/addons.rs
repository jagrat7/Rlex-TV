//! The Stremio addon protocol: manifests, catalogs, metadata and streams.
//!
//! <https://github.com/Stremio/stremio-addon-sdk/blob/master/docs/protocol.md>
//!
//! Addon JSON varies a lot between implementations, so every field is read
//! leniently and a malformed item is skipped rather than failing its list.

use std::time::Duration;

use anyhow::{Context as _, bail};
use serde::de::{DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Stremio's own metadata addon. It answers catalogs, search and metadata for
/// IMDb ids and needs no API key. It is a metadata provider, not a source:
/// it never answers `/stream/`.
pub const CINEMETA_URL: &str = "https://v3-cinemeta.strem.io";

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub version: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub logo: Option<String>,
    #[serde(default, deserialize_with = "lenient_list")]
    pub resources: Vec<Resource>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub types: Vec<String>,
    #[serde(default, deserialize_with = "lenient_list")]
    pub catalogs: Vec<CatalogDef>,
    #[serde(default, deserialize_with = "lenient_opt_strings")]
    pub id_prefixes: Option<Vec<String>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Resource {
    Name(String),
    Full {
        name: String,
        #[serde(default, deserialize_with = "lenient_opt_strings")]
        types: Option<Vec<String>>,
        #[serde(
            default,
            rename = "idPrefixes",
            deserialize_with = "lenient_opt_strings"
        )]
        id_prefixes: Option<Vec<String>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogDef {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient_list")]
    pub extra: Vec<ExtraDef>,
    /// The older way of declaring required extras.
    #[serde(default, deserialize_with = "lenient_strings")]
    pub extra_required: Vec<String>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub extra_supported: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtraDef {
    pub name: String,
    #[serde(default)]
    pub is_required: bool,
}

impl CatalogDef {
    pub fn supports_extra(&self, name: &str) -> bool {
        self.extra.iter().any(|e| e.name == name) || self.extra_supported.iter().any(|e| e == name)
    }

    /// Catalogs that need an extra (search, a genre…) can't be listed on their own.
    pub fn needs_extra(&self) -> bool {
        self.extra.iter().any(|e| e.is_required) || !self.extra_required.is_empty()
    }
}

/// An installed addon: where it lives and what it said it can do.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Addon {
    /// Without the trailing `/manifest.json`; configuration in the path is kept.
    pub base_url: String,
    pub manifest: Manifest,
}

impl Addon {
    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    /// Whether this addon answers `resource` requests for a `kind` item with this id.
    pub fn provides(&self, resource: &str, kind: &str, id: &str) -> bool {
        self.manifest.resources.iter().any(|r| {
            let (name, types, prefixes) = match r {
                Resource::Name(name) => (name, None, None),
                Resource::Full {
                    name,
                    types,
                    id_prefixes,
                } => (name, types.as_ref(), id_prefixes.as_ref()),
            };
            let types = types.unwrap_or(&self.manifest.types);
            let prefixes = prefixes.or(self.manifest.id_prefixes.as_ref());
            name == resource
                && types.iter().any(|t| t == kind)
                && prefixes.is_none_or(|p| p.iter().any(|p| id.starts_with(p.as_str())))
        })
    }
}

/// Turns what a user pasted (`stremio://…`, `…/manifest.json`, a bare host)
/// into an addon base URL.
pub fn normalize_url(input: &str) -> anyhow::Result<String> {
    let input = input.trim();
    let with_scheme = if let Some(rest) = input.strip_prefix("stremio://") {
        format!("https://{rest}")
    } else if input.contains("://") {
        input.to_string()
    } else {
        format!("https://{input}")
    };
    let base = with_scheme
        .strip_suffix("/manifest.json")
        .unwrap_or(&with_scheme)
        .trim_end_matches('/');
    let url = url::Url::parse(base).with_context(|| format!("“{input}” isn't a URL"))?;
    if !matches!(url.scheme(), "http" | "https") {
        bail!("addon URLs start with https:// or stremio://");
    }
    Ok(base.to_string())
}

/// A title, as listed in a catalog or described in full by `/meta/`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Meta {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub name: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub poster: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub background: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub logo: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub release_info: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub year: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub imdb_rating: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub runtime: Option<String>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub genres: Vec<String>,
    /// Cinemeta lists send `genre`, its metadata sends both.
    #[serde(default, deserialize_with = "lenient_strings")]
    pub genre: Vec<String>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub cast: Vec<String>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub director: Vec<String>,
    /// Episodes for a series; empty in catalog listings.
    #[serde(default, deserialize_with = "lenient_list")]
    pub videos: Vec<Video>,
}

impl Meta {
    pub fn genres(&self) -> &[String] {
        if self.genres.is_empty() {
            &self.genre
        } else {
            &self.genres
        }
    }

    /// `2008–2013`, `2014`, or nothing.
    pub fn year_label(&self) -> Option<&str> {
        self.release_info.as_deref().or(self.year.as_deref())
    }

    /// Seasons that have episodes, ascending, with specials (season 0) last.
    pub fn seasons(&self) -> Vec<u32> {
        let mut seasons: Vec<u32> = self.videos.iter().filter_map(|v| v.season).collect();
        seasons.sort_unstable_by_key(|&s| (s == 0, s));
        seasons.dedup();
        seasons
    }

    pub fn episodes(&self, season: u32) -> Vec<&Video> {
        let mut episodes: Vec<&Video> = self
            .videos
            .iter()
            .filter(|v| v.season == Some(season))
            .collect();
        episodes.sort_by_key(|v| v.episode_number());
        episodes
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Video {
    pub id: String,
    #[serde(default, deserialize_with = "lenient_string")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient_u32")]
    pub season: Option<u32>,
    #[serde(default, deserialize_with = "lenient_u32")]
    pub episode: Option<u32>,
    #[serde(default, deserialize_with = "lenient_u32")]
    pub number: Option<u32>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub released: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub thumbnail: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub overview: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub description: Option<String>,
}

impl Video {
    pub fn episode_number(&self) -> Option<u32> {
        self.episode.or(self.number)
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref().or(self.name.as_deref())
    }

    pub fn overview(&self) -> Option<&str> {
        self.overview.as_deref().or(self.description.as_deref())
    }

    /// `S1 E2 · Pilot`
    pub fn label(&self) -> String {
        let mut label = match (self.season, self.episode_number()) {
            (Some(s), Some(e)) => format!("S{s} E{e}"),
            (None, Some(e)) => format!("E{e}"),
            _ => String::new(),
        };
        if let Some(title) = self.title() {
            if !label.is_empty() {
                label.push_str(" · ");
            }
            label.push_str(title);
        }
        label
    }
}

/// One way to play a video: a direct URL (debrid, HTTP) or a torrent.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Stream {
    #[serde(default, deserialize_with = "lenient_string")]
    pub name: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub description: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub url: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub info_hash: Option<String>,
    #[serde(default, deserialize_with = "lenient_u32")]
    pub file_idx: Option<u32>,
    #[serde(default, deserialize_with = "lenient_strings")]
    pub sources: Vec<String>,
    #[serde(default, deserialize_with = "lenient_hints")]
    pub behavior_hints: BehaviorHints,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BehaviorHints {
    #[serde(default, deserialize_with = "lenient_string")]
    pub filename: Option<String>,
    #[serde(default, deserialize_with = "lenient_string")]
    pub binge_group: Option<String>,
}

impl Stream {
    /// Only URL and torrent streams can be played; YouTube ids and external
    /// web pages are skipped.
    pub fn is_playable(&self) -> bool {
        self.info_hash.is_some() || self.url.as_deref().is_some_and(|u| u.starts_with("http"))
    }

    pub fn is_torrent(&self) -> bool {
        self.info_hash.is_some()
    }

    pub fn trackers(&self) -> Vec<String> {
        self.sources
            .iter()
            .filter_map(|s| s.strip_prefix("tracker:"))
            .map(str::to_string)
            .collect()
    }

    pub fn dedupe_key(&self) -> String {
        match (&self.info_hash, &self.url) {
            (Some(hash), _) => format!("{}:{:?}", hash.to_lowercase(), self.file_idx),
            (None, Some(url)) => url.clone(),
            (None, None) => String::new(),
        }
    }

    /// All the descriptive text an addon put on the stream.
    pub fn text(&self) -> String {
        [&self.name, &self.title, &self.description]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn quality(&self) -> Option<&'static str> {
        let text = self.text().to_lowercase();
        [
            ("2160p", "4K"),
            ("4k", "4K"),
            ("1080p", "1080p"),
            ("720p", "720p"),
            ("480p", "480p"),
        ]
        .into_iter()
        .find(|(needle, _)| text.contains(needle))
        .map(|(_, label)| label)
    }
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
}

impl Client {
    pub fn new() -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("Rlex/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(20))
            .build()
            .expect("failed to build the HTTP client");
        Self { http }
    }

    async fn get_json(&self, url: &str) -> anyhow::Result<Value> {
        let response = self.http.get(url).send().await?;
        let status = response.status();
        if !status.is_success() {
            bail!("{status} from {url}");
        }
        Ok(response.json().await?)
    }

    pub async fn manifest(&self, base_url: &str) -> anyhow::Result<Manifest> {
        let json = self.get_json(&format!("{base_url}/manifest.json")).await?;
        serde_json::from_value(json).context("not a Stremio addon manifest")
    }

    pub async fn catalog(
        &self,
        base_url: &str,
        kind: &str,
        id: &str,
        extra: &[(&str, &str)],
    ) -> anyhow::Result<Vec<Meta>> {
        let mut url = format!("{base_url}/catalog/{}/{}", segment(kind), segment(id));
        if !extra.is_empty() {
            let pairs: Vec<String> = extra
                .iter()
                .map(|(k, v)| format!("{k}={}", urlencoding::encode(v)))
                .collect();
            url.push('/');
            url.push_str(&pairs.join("&"));
        }
        url.push_str(".json");
        let json = self.get_json(&url).await?;
        Ok(items(json.get("metas")))
    }

    pub async fn meta(&self, base_url: &str, kind: &str, id: &str) -> anyhow::Result<Meta> {
        let url = format!("{base_url}/meta/{}/{}.json", segment(kind), segment(id));
        let mut json = self.get_json(&url).await?;
        let meta = json.get_mut("meta").map(Value::take).unwrap_or_default();
        serde_json::from_value(meta).with_context(|| format!("no metadata for {id}"))
    }

    pub async fn streams(
        &self,
        base_url: &str,
        kind: &str,
        video_id: &str,
    ) -> anyhow::Result<Vec<Stream>> {
        let url = format!(
            "{base_url}/stream/{}/{}.json",
            segment(kind),
            segment(video_id)
        );
        let json = self.get_json(&url).await?;
        let streams: Vec<Stream> = items(json.get("streams"));
        Ok(streams.into_iter().filter(Stream::is_playable).collect())
    }
}

/// Percent-encodes a path segment, keeping the `:` Stremio ids are built from.
fn segment(s: &str) -> String {
    urlencoding::encode(s).replace("%3A", ":")
}

/// Deserializes each element of a JSON array on its own, skipping the bad ones.
fn items<T: DeserializeOwned>(value: Option<&Value>) -> Vec<T> {
    let Some(Value::Array(values)) = value else {
        return Vec::new();
    };
    values
        .iter()
        .filter_map(|v| match serde_json::from_value(v.clone()) {
            Ok(item) => Some(item),
            Err(err) => {
                log::debug!("skipping malformed addon item: {err}");
                None
            }
        })
        .collect()
}

fn lenient_string<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(s) if !s.trim().is_empty() => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

fn lenient_strings<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Array(values) => values
            .into_iter()
            .filter_map(|v| match v {
                Value::String(s) => Some(s),
                _ => None,
            })
            .collect(),
        Value::String(s) => vec![s],
        _ => Vec::new(),
    })
}

fn lenient_opt_strings<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Vec<String>>, D::Error> {
    let value = Value::deserialize(d)?;
    if value.is_null() {
        return Ok(None);
    }
    lenient_strings(value)
        .map(Some)
        .map_err(serde::de::Error::custom)
}

fn lenient_u32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u32>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::Number(n) => n.as_u64().and_then(|n| u32::try_from(n).ok()),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    })
}

fn lenient_list<'de, D: Deserializer<'de>, T: DeserializeOwned>(d: D) -> Result<Vec<T>, D::Error> {
    Ok(items(Some(&Value::deserialize(d)?)))
}

fn lenient_hints<'de, D: Deserializer<'de>>(d: D) -> Result<BehaviorHints, D::Error> {
    Ok(serde_json::from_value(Value::deserialize(d)?).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_pasted_addon_urls() {
        assert_eq!(
            normalize_url("stremio://addon.example.com/cfg=abc/manifest.json").unwrap(),
            "https://addon.example.com/cfg=abc"
        );
        assert_eq!(
            normalize_url(" https://addon.example.com/ ").unwrap(),
            "https://addon.example.com"
        );
        assert_eq!(
            normalize_url("addon.example.com/manifest.json").unwrap(),
            "https://addon.example.com"
        );
        assert!(normalize_url("ftp://addon.example.com").is_err());
    }

    #[test]
    fn reads_resources_in_both_forms() {
        let manifest: Manifest = serde_json::from_value(serde_json::json!({
            "id": "x", "name": "X", "types": ["movie", "series"], "idPrefixes": ["tt"],
            "resources": ["catalog", {"name": "stream", "types": ["movie"], "idPrefixes": ["tt"]}],
            "catalogs": [{"type": "movie", "id": "top", "extra": [{"name": "search", "isRequired": true}]}, {"bad": 1}]
        }))
        .unwrap();
        let addon = Addon {
            base_url: "https://x".into(),
            manifest,
        };
        assert!(addon.provides("stream", "movie", "tt123"));
        assert!(!addon.provides("stream", "series", "tt123"));
        assert!(!addon.provides("stream", "movie", "kitsu:1"));
        assert!(addon.provides("catalog", "series", "tt1"));
        assert_eq!(addon.manifest.catalogs.len(), 1);
        assert!(addon.manifest.catalogs[0].needs_extra());
    }

    #[test]
    fn tolerates_odd_meta_fields() {
        let meta: Meta = serde_json::from_value(serde_json::json!({
            "id": "tt1", "type": "series", "name": "Show", "imdbRating": 7.5, "year": 2008,
            "genre": ["Drama"], "cast": "One Person",
            "videos": [{"id": "tt1:1:2", "season": "1", "number": 2, "name": "Two"}, {"season": 1}]
        }))
        .unwrap();
        assert_eq!(meta.imdb_rating.as_deref(), Some("7.5"));
        assert_eq!(meta.genres(), ["Drama"]);
        assert_eq!(meta.cast, ["One Person"]);
        assert_eq!(meta.videos.len(), 1);
        assert_eq!(meta.videos[0].label(), "S1 E2 · Two");
    }

    #[test]
    fn stream_helpers() {
        let stream: Stream = serde_json::from_value(serde_json::json!({
            "name": "Source\n1080p", "infoHash": "ABC", "fileIdx": 3,
            "sources": ["tracker:udp://t.example:80", "dht:ABC"],
            "behaviorHints": {"filename": "a.mkv", "bingeGroup": 5}
        }))
        .unwrap();
        assert!(stream.is_playable());
        assert_eq!(stream.trackers(), ["udp://t.example:80"]);
        assert_eq!(stream.quality(), Some("1080p"));
        assert_eq!(stream.behavior_hints.filename.as_deref(), Some("a.mkv"));
        assert_eq!(stream.dedupe_key(), "abc:Some(3)");

        let youtube: Stream = serde_json::from_value(serde_json::json!({"ytId": "abc"})).unwrap();
        assert!(!youtube.is_playable());
    }
}

/// Talks to the real Cinemeta: `cargo test -- --ignored live_cinemeta`.
#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "needs the network"]
    async fn live_cinemeta() {
        let client = Client::new();
        let top = client
            .catalog(CINEMETA_URL, "movie", "top", &[])
            .await
            .unwrap();
        assert!(top.len() > 10, "top movies: {}", top.len());

        let found = client
            .catalog(CINEMETA_URL, "movie", "top", &[("search", "the matrix")])
            .await
            .unwrap();
        assert!(found.iter().any(|m| m.id == "tt0133093"));

        let show = client
            .meta(CINEMETA_URL, "series", "tt0903747")
            .await
            .unwrap();
        assert_eq!(show.name, "Breaking Bad");
        assert_eq!(show.seasons().first(), Some(&1));
        assert_eq!(show.episodes(1)[0].label(), "S1 E1 · Pilot");
    }
}
