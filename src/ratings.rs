//! IMDb, Rotten Tomatoes and Metacritic scores from OMDb.
//!
//! <https://www.omdbapi.com/>. OMDb is metadata only and needs a key the user
//! supplies; Rlex ships with none. The free key allows 1000 requests a day, so
//! results are cached on disk for a week and only fetched for the title the
//! user is looking at, never for catalog rows.

use std::collections::HashMap;
use std::time::Duration;

use anyhow::bail;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::store;

/// How long fetched scores are trusted before asking OMDb again.
pub const TTL_SECS: u64 = 7 * 24 * 60 * 60;

/// Where users get a free key.
pub const KEY_URL: &str = "https://www.omdbapi.com/apikey.aspx";

/// A title with well-known scores everywhere, used to test a key.
const TEST_ID: &str = "tt0111161";

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

/// Whether OMDb can be asked about `id`: only IMDb title ids (`tt` and digits).
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
        let http = reqwest::Client::builder()
            .user_agent(concat!("Rlex/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(15))
            .build()
            .expect("failed to build the HTTP client");
        Self { http }
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
