//! Rotten Tomatoes and Metacritic scores (and IMDb's, when OMDb is asked).
//!
//! Wikidata is asked first: it needs no key and has both scores for most
//! well-known films, though rarely for shows. OMDb fills whatever Wikidata
//! lacks, using the user's key or the one built in with `RLEX_OMDB_KEY`.
//! The free OMDb key allows 1000 requests a day, so results are cached on
//! disk for a week and only fetched for the title the user is looking at,
//! never for catalog rows.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::bail;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store;

/// How long fetched scores are trusted before asking again.
pub const TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// How soon scores cached after a failed OMDb request are asked for again.
const RETRY_SECS: u64 = 60 * 60;

/// Where users get a free key.
pub const KEY_URL: &str = "https://www.omdbapi.com/apikey.aspx";

/// A title with well-known scores everywhere, used to test a key.
const TEST_ID: &str = "tt0111161";

/// The OMDb key built into this copy of Rlex, if it was built with
/// `RLEX_OMDB_KEY` set. The user's own key in Settings takes its place.
pub fn built_in_key() -> Option<&'static str> {
    option_env!("RLEX_OMDB_KEY")
        .map(str::trim)
        .filter(|k| !k.is_empty())
}

fn http_client() -> reqwest::Client {
    reqwest::Client::builder()
        // Wikimedia asks for a user agent that says who is calling.
        .user_agent(concat!(
            "Rlex/",
            env!("CARGO_PKG_VERSION"),
            " (https://github.com/jagrat7/Rlex-TV)"
        ))
        .timeout(Duration::from_secs(15))
        .build()
        .expect("failed to build the HTTP client")
}

/// Scores for one title. Missing values (OMDb's "N/A") are `None`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ratings {
    /// Out of ten, as OMDb writes it ("8.8").
    pub imdb: Option<String>,
    /// As OMDb writes it ("2,345,678").
    pub imdb_votes: Option<String>,
    /// Tomatometer percent.
    pub rotten_tomatoes: Option<u8>,
    /// Metascore, out of 100.
    pub metacritic: Option<u8>,
}

impl Ratings {
    /// Both scores Wikidata can give, so OMDb isn't needed.
    pub fn has_critics(&self) -> bool {
        self.rotten_tomatoes.is_some() && self.metacritic.is_some()
    }

    /// Fills the scores this one is missing from `other`.
    fn fill_from(&mut self, other: Ratings) {
        self.imdb = self.imdb.take().or(other.imdb);
        self.imdb_votes = self.imdb_votes.take().or(other.imdb_votes);
        self.rotten_tomatoes = self.rotten_tomatoes.or(other.rotten_tomatoes);
        self.metacritic = self.metacritic.or(other.metacritic);
    }
}

/// Whether scores can be looked up for `id`: only IMDb title ids (`tt` and digits).
pub fn is_imdb_id(id: &str) -> bool {
    id.strip_prefix("tt")
        .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()))
}

/// Why OMDb said no.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OmdbError {
    /// The key is wrong or not activated yet. Affects every request.
    InvalidKey(String),
    /// The daily quota is used up. Affects every request until tomorrow.
    LimitReached(String),
    /// Anything else OMDb reported.
    Other(String),
}

impl OmdbError {
    /// Errors about the key rather than the title: no point asking again
    /// with the same key.
    pub fn is_key_error(&self) -> bool {
        matches!(self, Self::InvalidKey(_) | Self::LimitReached(_))
    }
}

impl std::fmt::Display for OmdbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidKey(msg) => write!(f, "OMDb rejected the API key ({msg})"),
            Self::LimitReached(msg) => write!(f, "OMDb's daily limit is used up ({msg})"),
            Self::Other(msg) => write!(f, "OMDb: {msg}"),
        }
    }
}

impl std::error::Error for OmdbError {}

/// Reads an OMDb `?i=` response. A title OMDb doesn't know comes back as
/// empty ratings, which is worth caching; problems with the key are errors.
pub fn parse(body: &Value) -> Result<Ratings, OmdbError> {
    let field = |name: &str| body.get(name).and_then(Value::as_str).and_then(present);

    if body.get("Response").and_then(Value::as_str) == Some("False") {
        let error = field("Error").unwrap_or("unknown error").to_string();
        let lower = error.to_lowercase();
        return if lower.contains("api key") {
            Err(OmdbError::InvalidKey(error))
        } else if lower.contains("limit") {
            Err(OmdbError::LimitReached(error))
        } else if lower.contains("not found") || lower.contains("incorrect imdb id") {
            Ok(Ratings::default())
        } else {
            Err(OmdbError::Other(error))
        };
    }

    let listed = |source: &str| {
        body.get("Ratings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find(|r| r.get("Source").and_then(Value::as_str) == Some(source))
            .and_then(|r| r.get("Value").and_then(Value::as_str))
            .and_then(present)
    };

    Ok(Ratings {
        imdb: field("imdbRating")
            .or_else(|| listed("Internet Movie Database").and_then(|v| v.split('/').next()))
            .map(str::to_string),
        imdb_votes: field("imdbVotes").map(str::to_string),
        rotten_tomatoes: listed("Rotten Tomatoes").and_then(|v| percent(v.trim_end_matches('%'))),
        metacritic: field("Metascore")
            .or_else(|| listed("Metacritic").and_then(|v| v.split('/').next()))
            .and_then(percent),
    })
}

/// `None` for OMDb's "N/A" and blanks.
fn present(s: &str) -> Option<&str> {
    let s = s.trim();
    (!s.is_empty() && s != "N/A").then_some(s)
}

fn percent(s: &str) -> Option<u8> {
    s.trim().parse::<u8>().ok().filter(|n| *n <= 100)
}

#[derive(Clone)]
pub struct Omdb {
    http: reqwest::Client,
}

impl Default for Omdb {
    fn default() -> Self {
        Self::new()
    }
}

impl Omdb {
    pub fn new() -> Self {
        Self {
            http: http_client(),
        }
    }

    /// Scores for an IMDb id. Errors from OMDb itself come back as an
    /// [`OmdbError`] inside the `anyhow::Error`.
    pub async fn fetch(&self, key: &str, imdb_id: &str) -> anyhow::Result<Ratings> {
        if !is_imdb_id(imdb_id) {
            bail!("“{imdb_id}” isn't an IMDb id");
        }
        let url = format!(
            "https://www.omdbapi.com/?i={imdb_id}&apikey={}",
            urlencoding::encode(key.trim())
        );
        let response = self.http.get(url).send().await?;
        let status = response.status();
        // OMDb answers a bad key with 401 and a JSON body that says why.
        let body: Value = match response.json().await {
            Ok(body) => body,
            Err(_) if !status.is_success() => bail!("OMDb answered {status}"),
            Err(err) => return Err(err.into()),
        };
        Ok(parse(&body)?)
    }

    /// Checks a key with one request for a well-known title.
    pub async fn test(&self, key: &str) -> anyhow::Result<Ratings> {
        self.fetch(key, TEST_ID).await
    }
}

// Wikidata

const WIKIDATA_SPARQL: &str = "https://query.wikidata.org/sparql";
const ROTTEN_TOMATOES: &str = "Q105584";
const TOMATOMETER: &str = "Q108403393";
const METACRITIC: &str = "Q150248";
const METASCORE: &str = "Q106515043";

/// The SPARQL query for a title's review scores (P444) by Rotten Tomatoes or
/// Metacritic, with how each was measured (P459) and when (P585).
fn wikidata_query(imdb_id: &str) -> String {
    format!(
        "SELECT ?score ?by ?method ?date WHERE {{ \
         ?item wdt:P345 \"{imdb_id}\"; p:P444 ?st. \
         ?st ps:P444 ?score; pq:P447 ?by; wikibase:rank ?rank. \
         FILTER(?rank != wikibase:DeprecatedRank) \
         FILTER(?by IN (wd:{ROTTEN_TOMATOES}, wd:{METACRITIC})) \
         OPTIONAL {{ ?st pq:P459 ?method }} OPTIONAL {{ ?st pq:P585 ?date }} }}"
    )
}

/// Reads the query's results: the latest Tomatometer percent and Metascore.
/// Rotten Tomatoes' other numbers (average rating, audience score) are
/// skipped.
pub fn parse_wikidata(body: &Value) -> Ratings {
    struct Score<'a> {
        value: &'a str,
        by: &'a str,
        method: Option<&'a str>,
        date: &'a str,
    }
    let rows: Vec<Score> = body
        .pointer("/results/bindings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let get = |name: &str| {
                row.pointer(&format!("/{name}/value"))
                    .and_then(Value::as_str)
            };
            let qid = |name: &str| get(name).map(|uri| uri.rsplit('/').next().unwrap_or(uri));
            Some(Score {
                value: get("score")?.trim(),
                by: qid("by")?,
                method: qid("method"),
                date: get("date").unwrap_or(""),
            })
        })
        .collect();
    let latest = |by: &str, method: &str, read: fn(&str) -> Option<u8>| {
        rows.iter()
            .filter(|s| s.by == by && s.method.is_none_or(|m| m == method))
            .filter_map(|s| Some((s.date, read(s.value)?)))
            .max_by_key(|(date, _)| *date)
            .map(|(_, score)| score)
    };
    Ratings {
        rotten_tomatoes: latest(ROTTEN_TOMATOES, TOMATOMETER, |v| {
            percent(v.strip_suffix('%')?)
        }),
        metacritic: latest(METACRITIC, METASCORE, |v| match v.split_once('/') {
            Some((score, "100")) => percent(score),
            Some(_) => None,
            None => percent(v),
        }),
        ..Ratings::default()
    }
}

#[derive(Clone)]
pub struct Wikidata {
    http: reqwest::Client,
}

impl Default for Wikidata {
    fn default() -> Self {
        Self::new()
    }
}

impl Wikidata {
    pub fn new() -> Self {
        Self {
            http: http_client(),
        }
    }

    pub async fn fetch(&self, imdb_id: &str) -> anyhow::Result<Ratings> {
        if !is_imdb_id(imdb_id) {
            bail!("“{imdb_id}” isn't an IMDb id");
        }
        let url = format!(
            "{WIKIDATA_SPARQL}?format=json&query={}",
            urlencoding::encode(&wikidata_query(imdb_id))
        );
        let response = self
            .http
            .get(url)
            .header("Accept", "application/sparql-results+json")
            .send()
            .await?;
        let status = response.status();
        if !status.is_success() {
            bail!("Wikidata answered {status}");
        }
        let body: Value = response.json().await?;
        Ok(parse_wikidata(&body))
    }
}

// Both together

/// Scores from Wikidata, with OMDb asked for whatever it lacks.
pub struct Fetched {
    pub ratings: Ratings,
    /// OMDb was asked and failed; the scores are only Wikidata's.
    pub omdb_error: Option<anyhow::Error>,
}

#[derive(Clone, Default)]
pub struct Sources {
    wikidata: Wikidata,
    omdb: Omdb,
}

impl Sources {
    pub fn new() -> Self {
        Self {
            wikidata: Wikidata::new(),
            omdb: Omdb::new(),
        }
    }

    /// Wikidata first; OMDb (with `omdb_key`) when Wikidata is missing a
    /// score. An error only when nothing answered.
    pub async fn fetch(&self, omdb_key: Option<&str>, imdb_id: &str) -> anyhow::Result<Fetched> {
        let wikidata = self.wikidata.fetch(imdb_id).await;
        if let Err(err) = &wikidata {
            log::warn!("ratings: Wikidata: {err:#}");
        }
        let mut ratings = wikidata.as_ref().cloned().unwrap_or_default();
        let Some(key) = omdb_key.filter(|_| !ratings.has_critics()) else {
            return wikidata.map(|ratings| Fetched {
                ratings,
                omdb_error: None,
            });
        };
        match self.omdb.fetch(key, imdb_id).await {
            Ok(omdb) => {
                ratings.fill_from(omdb);
                Ok(Fetched {
                    ratings,
                    omdb_error: None,
                })
            }
            Err(err) if wikidata.is_ok() => Ok(Fetched {
                ratings,
                omdb_error: Some(err),
            }),
            Err(err) => Err(err),
        }
    }

    /// Checks an OMDb key with one request.
    pub async fn test_omdb(&self, key: &str) -> anyhow::Result<Ratings> {
        self.omdb.test(key).await
    }
}

/// Fetched scores and when they were fetched.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CachedRatings {
    pub fetched_at: u64,
    pub ratings: Ratings,
}

impl CachedRatings {
    pub fn is_fresh(&self, now: u64) -> bool {
        now.saturating_sub(self.fetched_at) < TTL_SECS
    }
}

/// `ratings.json`: scores by IMDb id.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct RatingsCache {
    pub entries: HashMap<String, CachedRatings>,
}

impl RatingsCache {
    pub const FILE: &str = "ratings.json";

    /// The saved cache, without entries too old to use.
    pub fn load() -> Self {
        let mut cache: Self = store::load(Self::FILE);
        let now = store::now();
        cache.entries.retain(|_, e| e.is_fresh(now));
        cache
    }

    pub fn save(&self) {
        if let Err(err) = store::save(Self::FILE, self) {
            log::error!("saving ratings: {err:#}");
        }
    }

    pub fn get(&self, imdb_id: &str) -> Option<&CachedRatings> {
        self.entries.get(imdb_id)
    }

    /// Caches scores that are missing what a failed OMDb request would have
    /// added, so they're asked for again within the hour.
    pub fn insert_briefly(&mut self, imdb_id: &str, ratings: Ratings) {
        let now = store::now();
        self.entries.insert(
            imdb_id.to_string(),
            CachedRatings {
                fetched_at: now.saturating_sub(TTL_SECS - RETRY_SECS),
                ratings,
            },
        );
    }

    /// Forgets titles missing a critics' score, for a new key to fill in.
    pub fn drop_incomplete(&mut self) {
        self.entries.retain(|_, e| e.ratings.has_critics());
    }

    pub fn insert(&mut self, imdb_id: &str, ratings: Ratings) {
        self.entries.insert(
            imdb_id.to_string(),
            CachedRatings {
                fetched_at: store::now(),
                ratings,
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_a_full_response() {
        let body = json!({
            "Title": "Inception", "Year": "2010", "imdbID": "tt1375666",
            "Ratings": [
                {"Source": "Internet Movie Database", "Value": "8.8/10"},
                {"Source": "Rotten Tomatoes", "Value": "87%"},
                {"Source": "Metacritic", "Value": "74/100"}
            ],
            "Metascore": "74", "imdbRating": "8.8", "imdbVotes": "2,610,021",
            "Response": "True"
        });
        assert_eq!(
            parse(&body).unwrap(),
            Ratings {
                imdb: Some("8.8".into()),
                imdb_votes: Some("2,610,021".into()),
                rotten_tomatoes: Some(87),
                metacritic: Some(74),
            }
        );
    }

    #[test]
    fn treats_na_as_missing() {
        let body = json!({
            "Title": "Obscure", "Ratings": [{"Source": "Rotten Tomatoes", "Value": "N/A"}],
            "Metascore": "N/A", "imdbRating": "N/A", "imdbVotes": "N/A", "Response": "True"
        });
        let ratings = parse(&body).unwrap();
        assert_eq!(ratings, Ratings::default());
    }

    #[test]
    fn falls_back_to_the_ratings_list() {
        let body = json!({
            "Ratings": [
                {"Source": "Internet Movie Database", "Value": "7.1/10"},
                {"Source": "Metacritic", "Value": "58/100"}
            ],
            "Response": "True"
        });
        let ratings = parse(&body).unwrap();
        assert_eq!(ratings.imdb.as_deref(), Some("7.1"));
        assert_eq!(ratings.metacritic, Some(58));
        assert_eq!(ratings.rotten_tomatoes, None);
    }

    #[test]
    fn invalid_key_is_an_error() {
        let body = json!({"Response": "False", "Error": "Invalid API key!"});
        let err = parse(&body).unwrap_err();
        assert!(matches!(err, OmdbError::InvalidKey(_)));
        assert!(err.is_key_error());
    }

    #[test]
    fn daily_limit_is_a_key_error() {
        let body = json!({"Response": "False", "Error": "Request limit reached!"});
        assert!(matches!(
            parse(&body).unwrap_err(),
            OmdbError::LimitReached(_)
        ));
    }

    #[test]
    fn unknown_title_is_empty() {
        let body = json!({"Response": "False", "Error": "Incorrect IMDb ID."});
        assert_eq!(parse(&body).unwrap(), Ratings::default());
        let body = json!({"Response": "False", "Error": "Movie not found!"});
        assert_eq!(parse(&body).unwrap(), Ratings::default());
    }

    fn binding(score: &str, by: &str, method: Option<&str>, date: &str) -> Value {
        let entity = |q: &str| json!({"type": "uri", "value": format!("http://www.wikidata.org/entity/{q}")});
        let mut row = json!({
            "score": {"type": "literal", "value": score},
            "by": entity(by),
            "date": {"type": "literal", "value": date},
        });
        if let Some(method) = method {
            row["method"] = entity(method);
        }
        row
    }

    #[test]
    fn reads_wikidata_scores() {
        let body = json!({"results": {"bindings": [
            binding("7.1/10", ROTTEN_TOMATOES, Some("Q108403540"), "2021-10-09T00:00:00Z"),
            binding("72%", ROTTEN_TOMATOES, Some(TOMATOMETER), "2021-10-09T00:00:00Z"),
            binding("73%", ROTTEN_TOMATOES, Some(TOMATOMETER), "2025-10-08T00:00:00Z"),
            binding("74/100", METACRITIC, Some(METASCORE), "2025-06-16T00:00:00Z"),
        ]}});
        assert_eq!(
            parse_wikidata(&body),
            Ratings {
                rotten_tomatoes: Some(73),
                metacritic: Some(74),
                ..Ratings::default()
            }
        );
    }

    #[test]
    fn skips_other_rotten_tomatoes_numbers() {
        // An audience score (any method that isn't the Tomatometer) and a
        // Metacritic score out of something other than 100.
        let body = json!({"results": {"bindings": [
            binding("95%", ROTTEN_TOMATOES, Some("Q999"), "2025-01-01T00:00:00Z"),
            binding("8/10", METACRITIC, None, "2025-01-01T00:00:00Z"),
            binding("81", METACRITIC, None, ""),
        ]}});
        let ratings = parse_wikidata(&body);
        assert_eq!(ratings.rotten_tomatoes, None);
        assert_eq!(ratings.metacritic, Some(81));
        assert_eq!(parse_wikidata(&json!({})), Ratings::default());
    }

    #[test]
    fn omdb_only_fills_gaps() {
        let mut ratings = Ratings {
            rotten_tomatoes: Some(90),
            ..Ratings::default()
        };
        assert!(!ratings.has_critics());
        ratings.fill_from(Ratings {
            imdb: Some("8.1".into()),
            imdb_votes: None,
            rotten_tomatoes: Some(88),
            metacritic: Some(70),
        });
        assert_eq!(ratings.rotten_tomatoes, Some(90));
        assert_eq!(ratings.metacritic, Some(70));
        assert_eq!(ratings.imdb.as_deref(), Some("8.1"));
        assert!(ratings.has_critics());
    }

    #[tokio::test]
    #[ignore = "needs the network"]
    async fn live_wikidata() {
        let wikidata = Wikidata::new();
        let interstellar = wikidata.fetch("tt0816692").await.unwrap();
        assert!(interstellar.rotten_tomatoes.is_some(), "{interstellar:?}");
        assert!(interstellar.metacritic.is_some(), "{interstellar:?}");
        // Shows are rarely there; the answer is just empty.
        let show = Sources::new().fetch(None, "tt0903747").await.unwrap();
        assert!(show.omdb_error.is_none());
        println!(
            "interstellar {interstellar:?}, breaking bad {:?}",
            show.ratings
        );
    }

    #[test]
    fn recognizes_imdb_ids() {
        assert!(is_imdb_id("tt0111161"));
        assert!(!is_imdb_id("tt"));
        assert!(!is_imdb_id("local:abc"));
        assert!(!is_imdb_id("tt0903747:1:2"));
        assert!(!is_imdb_id("kitsu:1"));
    }

    #[test]
    fn cache_entries_expire() {
        let entry = CachedRatings {
            fetched_at: 1_000,
            ratings: Ratings::default(),
        };
        assert!(entry.is_fresh(1_000 + TTL_SECS - 1));
        assert!(!entry.is_fresh(1_000 + TTL_SECS));
    }
}
