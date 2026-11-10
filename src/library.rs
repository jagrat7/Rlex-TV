//! The local library: video files in the user's folders, named the way
//! release groups and Plex users name them, matched to Cinemeta titles.

use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::UNIX_EPOCH;

use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::addons::{self, CINEMETA_URL, Meta};

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "webm", "wmv", "ts", "m2ts", "mpg", "mpeg", "flv", "ogv",
];

/// Files named like a sample and smaller than this are skipped.
const SAMPLE_MAX_BYTES: u64 = 300 * 1024 * 1024;

/// What a file name says about its contents.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Parsed {
    pub title: String,
    pub year: Option<u16>,
    pub season: Option<u32>,
    pub episode: Option<u32>,
    /// The other name the file's context gives it (the file name when the
    /// title came from the show folder, or the other way round). Looked up
    /// when `title` finds nothing.
    pub alt_title: Option<String>,
}

impl Parsed {
    pub fn kind(&self) -> &'static str {
        if self.episode.is_some() {
            "series"
        } else {
            "movie"
        }
    }
}

/// How good a copy is, as far as its name says.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Quality {
    /// Vertical resolution: 2160, 1080, 720…; 0 when the name doesn't say.
    pub resolution: u16,
    pub source: Option<Source>,
    pub codec: Option<Codec>,
    pub hdr: bool,
}

/// Where a release came from, worst to best.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Source {
    Dvd,
    Hdtv,
    WebRip,
    WebDl,
    BluRay,
    Remux,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec {
    Xvid,
    H264,
    Hevc,
    Av1,
}

impl Quality {
    /// `1080p · BluRay · HEVC · HDR`; empty when the name says nothing.
    pub fn label(&self) -> String {
        let resolution = match self.resolution {
            0 => None,
            2160 => Some("4K".to_string()),
            r => Some(format!("{r}p")),
        };
        let source = self.source.map(|s| match s {
            Source::Dvd => "DVD",
            Source::Hdtv => "HDTV",
            Source::WebRip => "WEBRip",
            Source::WebDl => "WEB-DL",
            Source::BluRay => "BluRay",
            Source::Remux => "Remux",
        });
        let codec = self.codec.map(|c| match c {
            Codec::Xvid => "XviD",
            Codec::H264 => "H.264",
            Codec::Hevc => "HEVC",
            Codec::Av1 => "AV1",
        });
        let hdr = self.hdr.then_some("HDR");
        resolution
            .into_iter()
            .chain(
                [source, codec, hdr]
                    .into_iter()
                    .flatten()
                    .map(str::to_string),
            )
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// Fields this one lacks, taken from `other`.
    fn or(self, other: Quality) -> Quality {
        Quality {
            resolution: if self.resolution > 0 {
                self.resolution
            } else {
                other.resolution
            },
            source: self.source.or(other.source),
            codec: self.codec.or(other.codec),
            hdr: self.hdr || other.hdr,
        }
    }
}

/// The Cinemeta title a file belongs to.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Match {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub poster: Option<String>,
    pub background: Option<String>,
    pub year: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LocalFile {
    pub path: PathBuf,
    pub size: u64,
    /// Unix seconds.
    pub modified: u64,
    pub parsed: Parsed,
    #[serde(default)]
    pub quality: Quality,
    /// The files that are the same title by name share a cluster; see [`triage`].
    #[serde(default)]
    pub cluster: String,
    pub matched: Option<Match>,
}

impl LocalFile {
    /// `tt123` when matched, else `local:<cluster>`.
    pub fn meta_id(&self) -> String {
        match &self.matched {
            Some(m) => m.id.clone(),
            None if self.cluster.is_empty() => format!("local:file:{}", self.path.display()),
            None => format!("local:{}", self.cluster),
        }
    }

    /// The Stremio video id: `tt123` for a movie, `tt123:1:2` for an episode.
    /// Copies of the same video share it.
    pub fn video_id(&self) -> String {
        match self.parsed.episode {
            Some(episode) => format!(
                "{}:{}:{episode}",
                self.meta_id(),
                self.parsed.season.unwrap_or(1)
            ),
            None => self.meta_id(),
        }
    }

    pub fn file_name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// `S1 E2`, for episodes.
    pub fn episode_label(&self) -> Option<String> {
        let episode = self.parsed.episode?;
        Some(match self.parsed.season {
            Some(season) => format!("S{season} E{episode}"),
            None => format!("E{episode}"),
        })
    }

    /// Orders copies of the same video: resolution, then source, then size.
    fn rank(&self) -> (u16, Option<Source>, u64) {
        (self.quality.resolution, self.quality.source, self.size)
    }
}

/// Sorts copies of a video best first.
fn best_first(files: &mut [LocalFile]) {
    files.sort_by_key(|f| Reverse(f.rank()));
}

/// A title in the library with all the files that belong to it, from
/// however many folders.
#[derive(Clone, Debug)]
pub struct LibraryTitle {
    /// The matched id, or `local:<cluster>`.
    pub meta_id: String,
    pub kind: String,
    pub name: String,
    pub poster: Option<String>,
    pub year: Option<String>,
    /// In episode order; copies of one video best first.
    pub files: Vec<LocalFile>,
    /// Newest file's modification time.
    pub added: u64,
}

/// One movie or episode on disk, with every copy of it.
#[derive(Clone, Debug)]
pub struct LocalVideo {
    pub video_id: String,
    pub season: Option<u32>,
    /// Best first. The rest are other sources for the same video.
    pub versions: Vec<LocalFile>,
}

impl LibraryTitle {
    /// The title's movie or episodes, in order, each with its copies.
    pub fn videos(&self) -> Vec<LocalVideo> {
        let mut videos: Vec<LocalVideo> = Vec::new();
        for file in &self.files {
            let video_id = file.video_id();
            match videos.iter_mut().find(|v| v.video_id == video_id) {
                Some(video) => video.versions.push(file.clone()),
                None => videos.push(LocalVideo {
                    video_id,
                    season: file.parsed.season,
                    versions: vec![file.clone()],
                }),
            }
        }
        videos
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Library {
    pub files: Vec<LocalFile>,
    /// Clusters Cinemeta had no answer for, so rescans don't keep asking.
    pub misses: HashSet<String>,
}

impl Library {
    pub const FILE: &str = "library.json";

    /// Titles sorted by name. Files group by their match, so a show spread
    /// over several folders and naming styles is one title; unmatched files
    /// group by cluster.
    pub fn titles(&self) -> Vec<LibraryTitle> {
        let mut titles: HashMap<String, LibraryTitle> = HashMap::new();
        for file in &self.files {
            let title = titles
                .entry(file.meta_id())
                .or_insert_with(|| LibraryTitle {
                    meta_id: file.meta_id(),
                    kind: file
                        .matched
                        .as_ref()
                        .map_or(file.parsed.kind().to_string(), |m| m.kind.clone()),
                    name: file.matched.as_ref().map_or_else(
                        || {
                            Some(file.parsed.title.clone())
                                .filter(|t| !t.is_empty())
                                .unwrap_or_else(|| file.file_name())
                        },
                        |m| m.name.clone(),
                    ),
                    poster: file.matched.as_ref().and_then(|m| m.poster.clone()),
                    year: file.matched.as_ref().and_then(|m| m.year.clone()),
                    files: Vec::new(),
                    added: 0,
                });
            if title.year.is_none() {
                title.year = file.parsed.year.map(|y| y.to_string());
            }
            title.added = title.added.max(file.modified);
            title.files.push(file.clone());
        }
        let mut titles: Vec<LibraryTitle> = titles.into_values().collect();
        for title in &mut titles {
            best_first(&mut title.files);
            // Stable, so copies stay best first within each episode.
            title
                .files
                .sort_by_key(|f| (f.parsed.season.unwrap_or(1), f.parsed.episode));
        }
        titles.sort_by_key(|t| t.name.to_lowercase());
        titles
    }

    pub fn title(&self, meta_id: &str) -> Option<LibraryTitle> {
        self.titles().into_iter().find(|t| t.meta_id == meta_id)
    }

    pub fn recently_added(&self, limit: usize) -> Vec<LibraryTitle> {
        let mut titles = self.titles();
        titles.sort_by_key(|t| Reverse(t.added));
        titles.truncate(limit);
        titles
    }

    /// Files for a title, or for one of its videos when `video_id` is given;
    /// copies of a video best first.
    pub fn files_for(&self, meta_id: &str, video_id: Option<&str>) -> Vec<LocalFile> {
        let mut files: Vec<LocalFile> = self
            .files
            .iter()
            .filter(|f| match video_id {
                Some(video_id) => f.video_id() == video_id,
                None => f.meta_id() == meta_id,
            })
            .cloned()
            .collect();
        best_first(&mut files);
        files
    }
}

/// Walks the folders for video files and triages them. A file reached twice
/// (overlapping folders, symlinks, hardlinks) is listed once. A match from
/// `previous` is kept while the file and its cluster are unchanged, so a
/// better reading of its name gets a fresh lookup.
pub fn scan(folders: &[PathBuf], previous: &Library) -> Vec<LocalFile> {
    let known: HashMap<&Path, &LocalFile> = previous
        .files
        .iter()
        .map(|f| (f.path.as_path(), f))
        .collect();
    let mut files = Vec::new();
    let mut seen = HashSet::new();
    for folder in folders {
        let walker = walkdir::WalkDir::new(folder)
            .follow_links(true)
            .into_iter()
            .filter_entry(|e| e.depth() == 0 || !e.file_name().to_string_lossy().starts_with('.'));
        for entry in walker.filter_map(Result::ok) {
            let path = entry.path();
            if !entry.file_type().is_file() || !is_video(path) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if !seen.insert(file_identity(path, &metadata)) {
                continue;
            }
            let size = metadata.len();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.contains("sample") && size < SAMPLE_MAX_BYTES {
                continue;
            }
            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_secs());
            let matched = known
                .get(path)
                .filter(|f| f.size == size)
                .and_then(|f| f.matched.clone());
            files.push(LocalFile {
                path: path.to_path_buf(),
                size,
                modified,
                parsed: parse_path(path),
                quality: quality_of_path(path),
                cluster: String::new(),
                matched,
            });
        }
    }
    triage(&mut files);
    for file in &mut files {
        let same_cluster = known
            .get(file.path.as_path())
            .is_some_and(|f| f.cluster == file.cluster);
        if !same_cluster {
            file.matched = None;
        }
    }
    files
}

/// The same file however it's reached, hard links included: its inode on Unix, its volume
/// and file ID on Windows, else its real path.
fn file_identity(path: &Path, metadata: &std::fs::Metadata) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let _ = path;
        format!("{}:{}", metadata.dev(), metadata.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        #[cfg(windows)]
        if let Some(id) = windows_file_id(path) {
            return id;
        }
        std::fs::canonicalize(path)
            .unwrap_or_else(|_| path.to_path_buf())
            .display()
            .to_string()
    }
}

/// A file's volume and full 128-bit file ID, the same for every hard link to it. The older
/// 64-bit file index isn't unique on ReFS (Dev Drives), where it could merge two videos.
#[cfg(windows)]
fn windows_file_id(path: &Path) -> Option<String> {
    use std::os::windows::{fs::OpenOptionsExt, io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ID_INFO, FileIdInfo, GetFileInformationByHandleEx,
    };
    // No access rights are needed to read a file's ID.
    let file = std::fs::OpenOptions::new().access_mode(0).open(path).ok()?;
    // SAFETY: FILE_ID_INFO is plain data, and the call writes at most its size.
    let mut info: FILE_ID_INFO = unsafe { std::mem::zeroed() };
    let ok = unsafe {
        GetFileInformationByHandleEx(
            file.as_raw_handle(),
            FileIdInfo,
            (&raw mut info).cast(),
            size_of::<FILE_ID_INFO>() as u32,
        )
    };
    (ok != 0).then(|| {
        let id = u128::from_le_bytes(info.FileId.Identifier);
        format!("{}:{id:x}", info.VolumeSerialNumber)
    })
}

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Groups files that are the same title by name, across folders and naming
/// styles, so each group is looked up once and listed as one title.
///
/// Files group by kind and normalized title. Years split a group only when
/// they disagree by more than one (Doctor Who 1963 and 2005); a file that
/// names no year joins its title's largest group.
pub fn triage(files: &mut [LocalFile]) {
    let mut groups: HashMap<(&'static str, String), Vec<usize>> = HashMap::new();
    for (i, file) in files.iter_mut().enumerate() {
        let title = normalize(&file.parsed.title);
        if title.is_empty() {
            // Nothing to go on: a group of one.
            file.cluster = format!("file:{}", file.path.display());
            continue;
        }
        groups
            .entry((file.parsed.kind(), title))
            .or_default()
            .push(i);
    }

    for ((kind, title), members) in groups {
        let mut years: Vec<u16> = members
            .iter()
            .filter_map(|&i| files[i].parsed.year)
            .collect();
        years.sort_unstable();
        years.dedup();
        // Runs of years at most one apart, as (first, last).
        let mut eras: Vec<(u16, u16)> = Vec::new();
        for year in years {
            match eras.last_mut() {
                Some((_, last)) if year - *last <= 1 => *last = year,
                _ => eras.push((year, year)),
            }
        }
        let era_of = |year: u16| {
            eras.iter()
                .position(|&(first, last)| (first..=last).contains(&year))
        };
        let mut sizes = vec![0usize; eras.len()];
        for &i in &members {
            if let Some(era) = files[i].parsed.year.and_then(era_of) {
                sizes[era] += 1;
            }
        }
        let largest = (0..eras.len()).max_by_key(|&e| (sizes[e], Reverse(e)));
        for &i in &members {
            let era = files[i].parsed.year.and_then(era_of).or(largest);
            files[i].cluster = match era {
                Some(era) => format!("{kind}:{title}:{}", eras[era].0),
                None => format!("{kind}:{title}"),
            };
        }
    }
}

/// Looks up unmatched clusters on Cinemeta, one query per cluster, using the
/// best clues among its files. A cluster that already has a match passes it
/// on to new files in it. Returns how many files were newly matched.
pub async fn match_files(client: &addons::Client, library: &mut Library) -> usize {
    let mut found: HashMap<String, Match> = HashMap::new();
    for file in &library.files {
        if let Some(m) = &file.matched {
            found
                .entry(file.cluster.clone())
                .or_insert_with(|| m.clone());
        }
    }

    struct Lookup {
        kind: &'static str,
        year: Option<u16>,
        titles: Vec<String>,
    }
    let mut wanted: HashMap<String, Lookup> = HashMap::new();
    for file in &library.files {
        let cluster = &file.cluster;
        if file.matched.is_some()
            || found.contains_key(cluster)
            || library.misses.contains(cluster)
            || cluster.starts_with("file:")
        {
            continue;
        }
        let lookup = wanted.entry(cluster.clone()).or_insert_with(|| Lookup {
            kind: file.parsed.kind(),
            year: None,
            titles: Vec::new(),
        });
        lookup.year = lookup.year.or(file.parsed.year);
        for title in std::iter::once(&file.parsed.title).chain(&file.parsed.alt_title) {
            let key = normalize(title);
            if !key.is_empty() && !lookup.titles.iter().any(|t| normalize(t) == key) {
                lookup.titles.push(title.clone());
            }
        }
    }

    for (cluster, lookup) in wanted {
        let mut failed = false;
        for title in &lookup.titles {
            let results = client
                .catalog(
                    CINEMETA_URL,
                    lookup.kind,
                    "top",
                    &[("search", title.as_str())],
                )
                .await;
            let results = match results {
                Ok(results) => results,
                Err(err) => {
                    log::warn!("couldn't look up “{title}”: {err:#}");
                    failed = true;
                    continue;
                }
            };
            let wanted = Parsed {
                title: title.clone(),
                year: lookup.year,
                ..Default::default()
            };
            if let Some(meta) = best_match(&wanted, &results) {
                found.insert(
                    cluster.clone(),
                    Match {
                        id: meta.id.clone(),
                        kind: meta.kind.clone(),
                        name: meta.name.clone(),
                        poster: meta.poster.clone(),
                        background: meta.background.clone(),
                        year: meta.year_label().map(str::to_string),
                    },
                );
                break;
            }
        }
        // Network trouble isn't a miss: try again on the next scan.
        if !found.contains_key(&cluster) && !failed {
            library.misses.insert(cluster);
        }
    }

    let mut matched = 0;
    for file in library.files.iter_mut().filter(|f| f.matched.is_none()) {
        if let Some(m) = found.get(&file.cluster) {
            file.matched = Some(m.clone());
            matched += 1;
        }
    }
    matched
}

/// Picks the search result that is plainly the same title, or none.
fn best_match<'a>(parsed: &Parsed, results: &'a [Meta]) -> Option<&'a Meta> {
    let wanted = normalize(&parsed.title);
    let year_of = |meta: &Meta| -> Option<u16> {
        meta.year_label()
            .and_then(|y| y.get(..4))
            .and_then(|y| y.parse().ok())
    };
    let year_fits = |meta: &Meta| match (parsed.year, year_of(meta)) {
        (Some(want), Some(got)) => want.abs_diff(got) <= 1,
        _ => true,
    };
    let same_name = |meta: &Meta| normalize(&meta.name) == wanted;
    // One name inside the other, and not much longer: "Agents of SHIELD" is
    // "Marvel's Agents of S.H.I.E.L.D.", "Home Video" isn't "The McKinney
    // Family Home Video".
    let similar_name = |meta: &Meta| {
        let name = normalize(&meta.name);
        let (short, long) = if name.len() < wanted.len() {
            (&name, &wanted)
        } else {
            (&wanted, &name)
        };
        !short.is_empty() && long.contains(short.as_str()) && short.len() * 5 >= long.len() * 3
    };

    results
        .iter()
        .find(|m| same_name(m) && year_fits(m) && parsed.year.is_some())
        .or_else(|| results.iter().find(|m| same_name(m) && year_fits(m)))
        .or_else(|| {
            results
                .iter()
                .take(3)
                .find(|m| similar_name(m) && year_fits(m) && parsed.year.is_some())
        })
}

/// Lowercase letters and digits only, for comparing titles.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

static SXXEYY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bs(\d{1,2})[ ._-]?e(\d{1,3})").unwrap());
static NXNN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(\d{1,2})x(\d{2,3})\b").unwrap());
static SEASON_EPISODE_WORDS: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bseason\s*(\d{1,2})\s*,?\s*episode\s*(\d{1,3})\b").unwrap());
/// `Show - 01`, as anime releases number episodes.
static DASH_EPISODE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\s-\s(\d{1,3})(?:v\d)?(?:\s|$)").unwrap());
static YEAR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(19\d{2}|20\d{2})\b").unwrap());
static JUNK: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(concat!(
        // Only words that don't turn up in real titles ("web", "limited"…).
        r"(?i)\b(2160p|1080p|1080i|720p|576p|480p|4k|uhd|hdr|hdr10|dovi|bluray|blu-ray|bdrip|brrip|",
        r"webrip|web-dl|webdl|hdtv|dvdrip|remux|x264|x265|h264|h265|hevc|avc|",
        r"xvid|aac|ac3|eac3|dts|ddp|dd5|atmos|10bit|8bit|repack|unrated|remastered|",
        r"imax|amzn|dsnp|hmax|atvp)\b"
    ))
    .unwrap()
});
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\[[^\]]*\]|\{[^}]*\}").unwrap());
static SEASON_FOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:season|series|s)\s*(\d{1,2})$|^(specials)$").unwrap());
static LEADING_NUMBER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(?:e|ep|episode)?\s*(\d{1,3})\b").unwrap());

/// Parses a file in its folder context. Episodes in `Show/Season 1/` take
/// the show's name from its folder, as Plex does; `Movie (2010)/movie.mkv`
/// takes the movie's from its folder.
pub fn parse_path(path: &Path) -> Parsed {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let parent_name = name_of(path.parent());
    let grandparent_name = name_of(path.parent().and_then(Path::parent));
    let mut parsed = parse_name(&stem);

    if let Some(season) = season_of_folder(&parent_name) {
        if parsed.episode.is_none() {
            // `Season 1/03 - Title.mkv`
            let cleaned = clean(&stem);
            let Some(caps) = LEADING_NUMBER.captures(cleaned.trim()) else {
                return parsed;
            };
            parsed.episode = caps[1].parse().ok();
            parsed.title.clear();
            parsed.year = None;
        }
        parsed.season = parsed.season.or(Some(season));
        let show = parse_name(&grandparent_name);
        if !is_generic(&show.title) {
            let file_title = std::mem::replace(&mut parsed.title, show.title);
            parsed.year = show.year.or(parsed.year);
            parsed.alt_title = Some(file_title)
                .filter(|t| !is_generic(t) && normalize(t) != normalize(&parsed.title));
        }
        return parsed;
    }

    if parsed.episode.is_some() {
        // `Show/S01E02.mkv`
        let show = parse_name(&parent_name);
        if parsed.title.is_empty() && !is_generic(&show.title) {
            parsed.title = show.title;
            parsed.year = parsed.year.or(show.year);
        }
        return parsed;
    }

    let folder = parse_name(&parent_name);
    if parsed.year.is_none() && folder.year.is_some() && folder.episode.is_none() {
        let alt_title = Some(parsed.title)
            .filter(|t| !is_generic(t) && normalize(t) != normalize(&folder.title));
        return Parsed {
            alt_title,
            ..folder
        };
    }
    parsed
}

fn name_of(path: Option<&Path>) -> String {
    path.and_then(Path::file_name)
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Folder and file names that say nothing about the title.
fn is_generic(title: &str) -> bool {
    const GENERIC: &[&str] = &[
        "",
        "tv",
        "tvshows",
        "shows",
        "series",
        "television",
        "movies",
        "movie",
        "films",
        "film",
        "videos",
        "video",
        "media",
        "downloads",
        "download",
        "anime",
        "torrents",
        "plex",
        "library",
        "complete",
    ];
    GENERIC.contains(&normalize(title).as_str())
}

/// The file's quality from its name, filling gaps from its folder's
/// (`Inception (2010) [1080p BluRay]/Inception.mkv`).
fn quality_of_path(path: &Path) -> Quality {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    split_name(&stem)
        .1
        .or(split_name(&name_of(path.parent())).1)
}

fn season_of_folder(name: &str) -> Option<u32> {
    let caps = SEASON_FOLDER.captures(name.trim())?;
    if caps.get(2).is_some() {
        return Some(0);
    }
    caps[1].parse().ok()
}

/// Brackets dropped, separators turned into spaces.
fn clean(name: &str) -> String {
    let without_brackets = BRACKETS.replace_all(name, " ");
    let spaced: String = without_brackets
        .chars()
        .map(|c| {
            if matches!(c, '.' | '_' | '(' | ')') {
                ' '
            } else {
                c
            }
        })
        .collect();
    spaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Parses a release-style name: `Show.Name.S01E02.1080p.WEB-DL`,
/// `Movie Name (2010) [1080p]`, `[Group] Show - 03`.
pub fn parse_name(name: &str) -> Parsed {
    split_name(name).0
}

/// The name's title part, parsed, and the quality its release tags give.
/// Tags are only read after the title, so `Charlotte's Web` isn't a WEB release.
fn split_name(name: &str) -> (Parsed, Quality) {
    let s = clean(name);
    let mut cut = s.len();
    let mut season = None;
    let mut episode = None;

    let episode_match = [&*SXXEYY, &*NXNN, &*SEASON_EPISODE_WORDS]
        .into_iter()
        .filter_map(|re| re.captures(&s))
        .min_by_key(|caps| caps.get(0).map_or(usize::MAX, |m| m.start()));
    if let Some(caps) = episode_match {
        season = caps[1].parse().ok();
        episode = caps[2].parse().ok();
        cut = cut.min(caps.get(0).unwrap().start());
    } else if let Some(caps) = DASH_EPISODE.captures(&s) {
        episode = caps[1].parse().ok();
        cut = cut.min(caps.get(0).unwrap().start());
    }

    if let Some(junk) = JUNK.find(&s) {
        // A name that starts with a junk word is still a name.
        if junk.start() > 0 {
            cut = cut.min(junk.start());
        }
    }

    // The year is the last one before the cut, unless it starts the title (`1917`).
    let year_match = YEAR.find_iter(&s[..cut]).filter(|m| m.start() > 0).last();
    let year = year_match.and_then(|m| m.as_str().parse().ok());
    if let Some(m) = year_match {
        cut = cut.min(m.start());
    }

    let title = s[..cut]
        .trim_matches(|c: char| c.is_whitespace() || c == '-')
        .to_string();
    let tags: Vec<&str> = BRACKETS.find_iter(name).map(|m| m.as_str()).collect();
    let quality = parse_quality(&format!("{} {}", &s[cut..], tags.join(" ")));
    let parsed = Parsed {
        title,
        year,
        season,
        episode,
        alt_title: None,
    };
    (parsed, quality)
}

static RESOLUTION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(2160|1440|1080|720|576|480|360)[pi]\b|\b(4k|uhd)\b").unwrap());
static SOURCES: LazyLock<[(Regex, Source); 7]> = LazyLock::new(|| {
    let re = |pattern: &str| Regex::new(pattern).unwrap();
    [
        (re(r"\b(remux|bdremux)\b"), Source::Remux),
        (
            re(r"\b(blu-?ray|bd-?rip|br-?rip|bdmv|bd25|bd50)\b"),
            Source::BluRay,
        ),
        (re(r"\b(web-?dl)\b"), Source::WebDl),
        (re(r"\bweb-?rip\b"), Source::WebRip),
        (re(r"\b(hdtv|pdtv)\b"), Source::Hdtv),
        (re(r"\b(dvd-?rip|dvd|dvd5|dvd9)\b"), Source::Dvd),
        // Scene releases say just "WEB".
        (re(r"\bweb\b"), Source::WebDl),
    ]
});
static CODECS: LazyLock<[(Regex, Codec); 4]> = LazyLock::new(|| {
    let re = |pattern: &str| Regex::new(pattern).unwrap();
    [
        (re(r"\b(x265|h ?265|hevc)\b"), Codec::Hevc),
        (re(r"\bav1\b"), Codec::Av1),
        (re(r"\b(x264|h ?264|avc)\b"), Codec::H264),
        (re(r"\b(xvid|divx)\b"), Codec::Xvid),
    ]
});
static HDR: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(hdr|hdr10|hdr10plus|dv|dovi|dolby vision)\b").unwrap());

/// Reads release tags: resolution, source, codec, HDR.
fn parse_quality(tags: &str) -> Quality {
    let text: String = tags
        .to_lowercase()
        .chars()
        .map(|c| {
            if matches!(c, '.' | '_' | '[' | ']' | '(' | ')') {
                ' '
            } else {
                c
            }
        })
        .collect();
    let resolution = RESOLUTION
        .captures(&text)
        .map_or(0, |caps| match caps.get(1) {
            Some(r) => r.as_str().parse().unwrap_or(0),
            None => 2160,
        });
    Quality {
        resolution,
        source: SOURCES
            .iter()
            .find(|(re, _)| re.is_match(&text))
            .map(|(_, s)| *s),
        codec: CODECS
            .iter()
            .find(|(re, _)| re.is_match(&text))
            .map(|(_, c)| *c),
        hdr: HDR.is_match(&text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(title: &str, year: Option<u16>, season: Option<u32>, episode: Option<u32>) -> Parsed {
        Parsed {
            title: title.into(),
            year,
            season,
            episode,
            alt_title: None,
        }
    }

    #[test]
    fn parses_release_names() {
        assert_eq!(
            parse_name("Breaking.Bad.S01E02.720p.BluRay.x264-GRP"),
            p("Breaking Bad", None, Some(1), Some(2))
        );
        assert_eq!(
            parse_name("The.Matrix.1999.1080p.BluRay.x264"),
            p("The Matrix", Some(1999), None, None)
        );
        assert_eq!(
            parse_name("Blade Runner 2049 (2017) [2160p]"),
            p("Blade Runner 2049", Some(2017), None, None)
        );
        assert_eq!(parse_name("1917"), p("1917", None, None, None));
        assert_eq!(
            parse_name("1917.2019.1080p"),
            p("1917", Some(2019), None, None)
        );
        assert_eq!(
            parse_name("2001.A.Space.Odyssey.1968.REMASTERED"),
            p("2001 A Space Odyssey", Some(1968), None, None)
        );
        assert_eq!(
            parse_name("Doctor.Who.2005.S03E08.HDTV"),
            p("Doctor Who", Some(2005), Some(3), Some(8))
        );
        assert_eq!(
            parse_name("friends 2x05"),
            p("friends", None, Some(2), Some(5))
        );
        assert_eq!(
            parse_name("[SubsPlease] Sousou no Frieren - 03 (1080p) [ABCD1234]"),
            p("Sousou no Frieren", None, None, Some(3))
        );
        assert_eq!(
            parse_name("Show Name Season 2 Episode 10"),
            p("Show Name", None, Some(2), Some(10))
        );
    }

    #[test]
    fn reads_quality_from_release_tags() {
        let q = |name: &str| split_name(name).1;
        assert_eq!(
            q("The.Matrix.1999.2160p.UHD.BluRay.REMUX.HDR.HEVC").label(),
            "4K · Remux · HEVC · HDR"
        );
        assert_eq!(
            q("Show.S01E01.1080p.WEB.H264-GRP").label(),
            "1080p · WEB-DL · H.264"
        );
        assert_eq!(q("[SubsPlease] Frieren - 03 (720p) [ABCD]").resolution, 720);
        // Words in the title aren't tags.
        assert_eq!(q("Charlotte's Web 2006 DVDRip").source, Some(Source::Dvd));
        assert_eq!(q("Home Video").label(), "");
        assert_eq!(
            quality_of_path(Path::new(
                "/m/Inception (2010) [1080p BluRay]/Inception.mkv"
            ))
            .label(),
            "1080p · BluRay"
        );
    }

    #[test]
    fn uses_folders_for_context() {
        assert_eq!(
            parse_path(Path::new("/tv/The Wire/Season 2/03 - Hot Shots.mkv")),
            p("The Wire", None, Some(2), Some(3))
        );
        assert_eq!(
            parse_path(Path::new("/tv/The Wire (2002)/Season 01/S01E01.mkv")),
            p("The Wire", Some(2002), Some(1), Some(1))
        );
        assert_eq!(
            parse_path(Path::new("/tv/Show/Specials/01.mkv")),
            p("Show", None, Some(0), Some(1))
        );
        assert_eq!(
            parse_path(Path::new("/movies/Inception (2010)/movie.mkv")),
            p("Inception", Some(2010), None, None)
        );
        assert_eq!(
            parse_path(Path::new("/movies/Inception.2010.1080p.mkv")),
            p("Inception", Some(2010), None, None)
        );
        // The show folder names the show; the file's own title is the fallback.
        assert_eq!(
            parse_path(Path::new(
                "/tv/The Office (US)/Season 2/The.Office.S02E01.mkv"
            )),
            Parsed {
                alt_title: Some("The Office".into()),
                ..p("The Office US", None, Some(2), Some(1))
            }
        );
        // A generic folder says nothing.
        assert_eq!(
            parse_path(Path::new("/media/TV/Season 1/Breaking.Bad.S01E01.mkv")),
            p("Breaking Bad", None, Some(1), Some(1))
        );
    }

    fn file(path: &str, size: u64) -> LocalFile {
        LocalFile {
            path: path.into(),
            size,
            modified: 0,
            parsed: parse_path(Path::new(path)),
            quality: quality_of_path(Path::new(path)),
            cluster: String::new(),
            matched: None,
        }
    }

    #[test]
    fn triage_groups_one_show_across_folders_and_names() {
        let mut files = vec![
            file("/tv/Breaking Bad/Season 1/Breaking.Bad.S01E01.720p.mkv", 1),
            file("/downloads/breaking_bad_s01e02_1080p.mkv", 1),
            file("/downloads/Breaking Bad (2008) S01E03.mkv", 1),
            file("/tv/Doctor Who (1963)/Season 1/01.mkv", 1),
            file("/tv/Doctor Who (2005)/Season 1/01.mkv", 1),
            file("/tv/Doctor.Who.2005.S01E02.mkv", 1),
        ];
        triage(&mut files);
        let clusters: Vec<&str> = files.iter().map(|f| f.cluster.as_str()).collect();
        assert_eq!(clusters[0], "series:breakingbad:2008");
        assert_eq!(clusters[1], clusters[0]);
        assert_eq!(clusters[2], clusters[0]);
        assert_eq!(clusters[3], "series:doctorwho:1963");
        assert_eq!(clusters[4], "series:doctorwho:2005");
        assert_eq!(clusters[5], clusters[4]);
    }

    #[test]
    fn copies_of_a_video_are_versions_best_first() {
        let mut files = vec![
            file("/a/The.Matrix.1999.720p.WEB.mkv", 2_000),
            file(
                "/b/The Matrix (1999)/The.Matrix.1999.1080p.BluRay.x264.mkv",
                9_000,
            ),
            file("/c/The.Matrix.1999.1080p.WEBRip.mkv", 3_000),
            file("/c/The.Matrix.mkv", 1_000),
            file("/tv/Show/Season 1/Show.S01E01.720p.mkv", 1),
            file("/tv/Show/Season 1/Show.S01E01.1080p.mkv", 1),
            file("/tv/Show/Season 1/Show.S01E02.1080p.mkv", 1),
        ];
        triage(&mut files);
        let library = Library {
            files,
            misses: HashSet::new(),
        };
        let titles = library.titles();
        assert_eq!(titles.len(), 2, "one movie and one show");

        let matrix = titles.iter().find(|t| t.kind == "movie").unwrap();
        assert_eq!(matrix.meta_id, "local:movie:thematrix:1999");
        let videos = matrix.videos();
        assert_eq!(videos.len(), 1);
        let paths: Vec<&str> = videos[0]
            .versions
            .iter()
            .map(|f| f.path.to_str().unwrap())
            .collect();
        assert_eq!(
            paths,
            [
                "/b/The Matrix (1999)/The.Matrix.1999.1080p.BluRay.x264.mkv",
                "/c/The.Matrix.1999.1080p.WEBRip.mkv",
                "/a/The.Matrix.1999.720p.WEB.mkv",
                "/c/The.Matrix.mkv",
            ]
        );

        let show = titles.iter().find(|t| t.kind == "series").unwrap();
        let episodes = show.videos();
        assert_eq!(episodes.len(), 2);
        assert_eq!(episodes[0].versions.len(), 2);
        assert_eq!(episodes[0].versions[0].quality.resolution, 1080);
        let best = library.files_for(&show.meta_id, Some(&episodes[0].video_id));
        assert_eq!(best[0].quality.resolution, 1080);
    }

    #[test]
    fn scan_lists_a_file_once_however_it_is_reached() {
        let root = std::env::temp_dir().join(format!("rlex-dedupe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let movies = root.join("Movies");
        std::fs::create_dir_all(&movies).unwrap();
        let original = movies.join("The.Matrix.1999.1080p.mkv");
        std::fs::write(&original, b"x").unwrap();
        std::fs::hard_link(&original, root.join("hardlink.mkv")).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&original, root.join("symlink.mkv")).unwrap();
        std::fs::write(root.join("copy.1999.mkv"), b"x").unwrap();

        // `Movies` overlaps `root`; the hardlink and symlink are the same file.
        let files = scan(&[root.clone(), movies], &Library::default());
        let names: Vec<String> = files.iter().map(LocalFile::file_name).collect();
        assert_eq!(files.len(), 2, "{names:?}");
        assert!(names.contains(&"copy.1999.mkv".to_string()));
        let _ = std::fs::remove_dir_all(root);
    }

    fn meta(id: &str, name: &str, year: &str) -> Meta {
        serde_json::from_value(serde_json::json!({
            "id": id, "type": "movie", "name": name, "releaseInfo": year
        }))
        .unwrap()
    }

    #[test]
    fn matches_conservatively() {
        let results = vec![
            meta("tt1", "The Matrix Reloaded", "2003"),
            meta("tt0133093", "The Matrix", "1999"),
        ];
        let want = p("The Matrix", Some(1999), None, None);
        assert_eq!(best_match(&want, &results).unwrap().id, "tt0133093");

        let wrong_year = p("The Matrix", Some(2021), None, None);
        assert!(best_match(&wrong_year, &results).is_none());

        let unrelated = p("Home Video", None, None, None);
        assert!(best_match(&unrelated, &results).is_none());

        let longer = vec![meta("tt2", "The McKinney Family Home Video", "2024")];
        let home_video = p("Home Video", Some(2023), None, None);
        assert!(best_match(&home_video, &longer).is_none());

        let prefixed = vec![meta(
            "tt2364582",
            "Marvel's Agents of S.H.I.E.L.D.",
            "2013–2020",
        )];
        let shield = p("Agents of SHIELD", Some(2013), None, None);
        assert_eq!(best_match(&shield, &prefixed).unwrap().id, "tt2364582");
    }

    #[test]
    fn video_ids_follow_stremio() {
        let mut episode = file("/tv/X/Season 2/X.S02E05.mkv", 1);
        episode.matched = Some(Match {
            id: "tt9".into(),
            kind: "series".into(),
            name: "X".into(),
            poster: None,
            background: None,
            year: None,
        });
        assert_eq!(episode.video_id(), "tt9:2:5");
        assert_eq!(episode.episode_label().as_deref(), Some("S2 E5"));

        let mut unmatched = file("/tv/Y/Season 1/Y.S01E03.mkv", 1);
        triage(std::slice::from_mut(&mut unmatched));
        assert_eq!(unmatched.meta_id(), "local:series:y");
        assert_eq!(unmatched.video_id(), "local:series:y:1:3");
    }
}

/// Scans a temporary folder and matches it on the real Cinemeta:
/// `cargo test -- --ignored live_match`.
#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "needs the network"]
    async fn live_match() {
        let _ = env_logger::builder().is_test(true).try_init();
        let root = std::env::temp_dir().join(format!("rlex-library-{}", std::process::id()));
        for name in [
            "TV/Breaking Bad/Season 1/Breaking.Bad.S01E01.720p.BluRay.mkv",
            "TV/Breaking Bad/Season 1/Breaking.Bad.S01E02.720p.BluRay.mkv",
            "Movies/The.Matrix.1999.1080p.BluRay.x264.mkv",
            "Movies/Inception (2010)/Inception.mkv",
            "Home/Birthday Party 2023.mp4",
            "Movies/The.Matrix.1999.1080p.BluRay.x264.sample.mkv",
            // Another copy of S01E02, named and filed differently.
            "Downloads/breaking.bad.s01e02.1080p.web.h264.mkv",
        ] {
            let path = root.join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, b"").unwrap();
        }

        let mut library = Library {
            files: scan(std::slice::from_ref(&root), &Library::default()),
            misses: HashSet::new(),
        };
        assert_eq!(library.files.len(), 6, "the sample is skipped");
        let matched = match_files(&addons::Client::new(), &mut library).await;
        assert_eq!(matched, 5);

        let ids: HashMap<String, String> = library
            .files
            .iter()
            .map(|f| (f.file_name(), f.video_id()))
            .collect();
        assert_eq!(ids["Breaking.Bad.S01E02.720p.BluRay.mkv"], "tt0903747:1:2");
        assert_eq!(ids["The.Matrix.1999.1080p.BluRay.x264.mkv"], "tt0133093");
        assert_eq!(ids["Inception.mkv"], "tt1375666");
        assert!(ids["Birthday Party 2023.mp4"].starts_with("local:"));
        assert_eq!(library.misses.len(), 1);
        assert_eq!(library.titles().len(), 4);

        let episode = library.files_for("tt0903747", Some("tt0903747:1:2"));
        assert_eq!(episode.len(), 2, "both copies are sources for S01E02");
        assert_eq!(episode[0].quality.label(), "1080p · WEB-DL · H.264");
        let _ = std::fs::remove_dir_all(root);
    }
}
