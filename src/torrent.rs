//! The in-process BitTorrent engine (librqbit) and its local streaming server.
//!
//! A torrent stream is served over HTTP on localhost, so the system's video
//! player can open it like any URL. librqbit fetches the pieces the player
//! reads first, so playback starts long before the download finishes.

use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(test)]
use std::time::Duration;

use anyhow::{Context as _, bail};
use librqbit::api::TorrentIdOrHash;
use librqbit::http_api::{HttpApi, HttpApiOptions};
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, Api, ManagedTorrent, Session,
    SessionOptions, SessionPersistenceConfig, TorrentStatsState,
};
use librqbit_dualstack_sockets::{BindOpts, TcpListener};

use crate::library::is_video;
use crate::store;

pub struct Engine {
    session: Arc<Session>,
    port: u16,
}

/// A torrent's state, for the downloads page.
#[derive(Clone, Debug, PartialEq)]
pub struct TorrentInfo {
    pub id: usize,
    pub name: String,
    /// `initializing`, `live`, `paused` or `error`.
    pub state: String,
    pub error: Option<String>,
    /// 0.0–1.0 of the selected files.
    pub progress: f32,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    /// MiB/s.
    pub down_speed: f64,
    pub up_speed: f64,
    pub peers: u32,
    pub finished: bool,
    pub paused: bool,
}

impl Engine {
    /// Starts the session (resuming torrents from last time) and the local
    /// streaming server. Must run on the Tokio runtime.
    pub async fn start(download_dir: PathBuf) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&download_dir)
            .with_context(|| format!("can't create {}", download_dir.display()))?;
        let session = Session::new_with_opts(
            download_dir,
            SessionOptions {
                fastresume: true,
                persistence: Some(SessionPersistenceConfig::Json {
                    folder: Some(session_dir()),
                }),
                ..Default::default()
            },
        )
        .await
        .context("couldn't start the torrent engine")?;

        let listener = TcpListener::bind_tcp(
            SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
            BindOpts {
                request_dualstack: false,
                ..Default::default()
            },
        )?;
        let port = listener.bind_addr().port();
        // Read-only: other local programs can read streams, but not add or delete torrents.
        let server = HttpApi::new(
            Api::new(session.clone(), None, None),
            Some(HttpApiOptions {
                read_only: true,
                ..Default::default()
            }),
        )
        .make_http_api_and_run(listener, None);
        tokio::spawn(async move {
            if let Err(err) = server.await {
                log::error!("torrent streaming server stopped: {err:#}");
            }
        });
        log::info!("torrent streams served on 127.0.0.1:{port}");

        Ok(Arc::new(Self { session, port }))
    }

    /// Adds the torrent if needed, makes sure the wanted file is downloading,
    /// and returns a URL a player can stream it from.
    ///
    /// The file is `file_idx` when the source named one, else the one called
    /// `file_name`, else the largest video.
    pub async fn stream(
        &self,
        info_hash: &str,
        file_idx: Option<usize>,
        file_name: Option<&str>,
        trackers: &[String],
    ) -> anyhow::Result<String> {
        let info_hash = info_hash.to_lowercase();
        let handle = match self.find(&info_hash) {
            Some(handle) => handle,
            None => self.add(&info_hash, file_idx, file_name, trackers).await?,
        };
        handle.wait_until_initialized().await?;

        let files = handle.with_metadata(|m| {
            m.file_infos
                .iter()
                .map(|f| (f.relative_filename.clone(), f.len))
                .collect::<Vec<_>>()
        })?;
        let index = pick_file(&files, file_idx, file_name).context("the torrent has no files")?;

        let mut selected: HashSet<usize> = handle
            .only_files()
            .map(|only| only.into_iter().collect())
            .unwrap_or_else(|| (0..files.len()).collect());
        if selected.insert(index) {
            self.session.update_only_files(&handle, &selected).await?;
        }
        if handle.is_paused() {
            self.session.unpause(&handle).await?;
        }

        let file_name = files[index]
            .0
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("file-{index}"));
        Ok(format!(
            "http://127.0.0.1:{}/torrents/{}/stream/{index}/{}",
            self.port,
            handle.id(),
            urlencoding::encode(&file_name)
        ))
    }

    fn find(&self, info_hash: &str) -> Option<Arc<ManagedTorrent>> {
        self.session.with_torrents(|torrents| {
            torrents
                .map(|(_, handle)| handle)
                .find(|handle| handle.info_hash().as_string() == info_hash)
                .cloned()
        })
    }

    async fn add(
        &self,
        info_hash: &str,
        file_idx: Option<usize>,
        file_name: Option<&str>,
        trackers: &[String],
    ) -> anyhow::Result<Arc<ManagedTorrent>> {
        let mut magnet = format!("magnet:?xt=urn:btih:{info_hash}");
        for tracker in trackers {
            magnet.push_str("&tr=");
            magnet.push_str(&urlencoding::encode(tracker));
        }

        // Without a file index, read the file list first so a season pack
        // downloads only the episode being played.
        let (add, only_files) = match file_idx {
            Some(index) => (AddTorrent::from_url(magnet), vec![index]),
            None => {
                let listed = self
                    .session
                    .add_torrent(
                        AddTorrent::from_url(magnet),
                        Some(AddTorrentOptions {
                            list_only: true,
                            ..Default::default()
                        }),
                    )
                    .await?;
                let AddTorrentResponse::ListOnly(listed) = listed else {
                    bail!("the engine didn't list the torrent's files");
                };
                let files: Vec<(PathBuf, u64)> = listed
                    .info
                    .iter_file_details()
                    .map(|f| (f.filename.to_pathbuf(), f.len))
                    .collect();
                let index =
                    pick_file(&files, None, file_name).context("the torrent has no files")?;
                (AddTorrent::from_bytes(listed.torrent_bytes), vec![index])
            }
        };

        let response = self
            .session
            .add_torrent(
                add,
                Some(AddTorrentOptions {
                    only_files: Some(only_files),
                    overwrite: true,
                    ..Default::default()
                }),
            )
            .await?;
        response
            .into_handle()
            .context("the engine didn't start the torrent")
    }

    pub fn torrents(&self) -> Vec<TorrentInfo> {
        let mut list: Vec<TorrentInfo> = self.session.with_torrents(|torrents| {
            torrents
                .map(|(id, handle)| {
                    let stats = handle.stats();
                    let live = stats.live.as_ref();
                    TorrentInfo {
                        id,
                        name: handle
                            .name()
                            .unwrap_or_else(|| handle.info_hash().as_string()),
                        state: stats.state.to_string(),
                        error: stats.error.clone(),
                        progress: if stats.total_bytes > 0 {
                            stats.progress_bytes as f32 / stats.total_bytes as f32
                        } else {
                            0.0
                        },
                        downloaded_bytes: stats.progress_bytes,
                        total_bytes: stats.total_bytes,
                        down_speed: live.map_or(0.0, |l| l.download_speed.mbps),
                        up_speed: live.map_or(0.0, |l| l.upload_speed.mbps),
                        peers: live.map_or(0, |l| l.snapshot.peer_stats.live),
                        finished: stats.finished,
                        paused: matches!(stats.state, TorrentStatsState::Paused),
                    }
                })
                .collect()
        });
        list.sort_by_key(|t| std::cmp::Reverse(t.id));
        list
    }

    pub async fn pause(&self, id: usize) -> anyhow::Result<()> {
        let handle = self.handle(id)?;
        self.session.pause(&handle).await
    }

    pub async fn resume(&self, id: usize) -> anyhow::Result<()> {
        let handle = self.handle(id)?;
        self.session.unpause(&handle).await
    }

    pub async fn remove(&self, id: usize, delete_files: bool) -> anyhow::Result<()> {
        self.session
            .delete(TorrentIdOrHash::Id(id), delete_files)
            .await
    }

    fn handle(&self, id: usize) -> anyhow::Result<Arc<ManagedTorrent>> {
        self.session
            .get(TorrentIdOrHash::Id(id))
            .context("no such torrent")
    }
}

fn session_dir() -> PathBuf {
    store::data_dir().join("torrents")
}

/// Whether the last session left torrents to resume, so the engine should
/// start with the app rather than on the first torrent played.
pub fn has_saved_torrents() -> bool {
    std::fs::read(session_dir().join("session.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|session| Some(!session.get("torrents")?.as_object()?.is_empty()))
        .unwrap_or(false)
}

/// `file_idx` if it exists, else the file called `file_name`, else the largest video.
fn pick_file(
    files: &[(PathBuf, u64)],
    file_idx: Option<usize>,
    file_name: Option<&str>,
) -> Option<usize> {
    if let Some(index) = file_idx.filter(|&i| i < files.len()) {
        return Some(index);
    }
    if let Some(wanted) = file_name {
        let wanted = wanted.to_lowercase();
        let named = files.iter().position(|(path, _)| {
            path.file_name()
                .is_some_and(|n| n.to_string_lossy().to_lowercase() == wanted)
        });
        if named.is_some() {
            return named;
        }
    }
    let largest = |videos_only: bool| {
        files
            .iter()
            .enumerate()
            .filter(|(_, (path, _))| !videos_only || is_video(path))
            .max_by_key(|(_, (_, len))| *len)
            .map(|(i, _)| i)
    };
    largest(true).or_else(|| largest(false))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_right_file() {
        let files = vec![
            (PathBuf::from("Show/S01E01.mkv"), 900),
            (PathBuf::from("Show/S01E02.mkv"), 1000),
            (PathBuf::from("Show/extras.iso"), 5000),
            (PathBuf::from("Show/readme.nfo"), 1),
        ];
        assert_eq!(pick_file(&files, Some(0), None), Some(0));
        assert_eq!(pick_file(&files, Some(9), Some("s01e01.mkv")), Some(0));
        assert_eq!(pick_file(&files, None, None), Some(1));
        assert_eq!(pick_file(&[], None, None), None);
    }
}

/// Streams Big Buck Bunny (Blender Foundation, CC BY 3.0; the WebTorrent
/// sample torrent) through the engine: `cargo test -- --ignored live_stream`.
#[cfg(test)]
mod live_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "joins a public BitTorrent swarm"]
    async fn live_stream() {
        let dir = std::env::temp_dir().join(format!("rlex-live-{}", std::process::id()));
        // SAFETY: set before any other thread reads the environment.
        unsafe { std::env::set_var("XDG_DATA_HOME", dir.join("data")) };
        let engine = Engine::start(dir.join("downloads")).await.unwrap();
        let trackers = [
            "udp://tracker.opentrackr.org:1337/announce".to_string(),
            "wss://tracker.webtorrent.dev".to_string(),
        ];
        let url = tokio::time::timeout(
            Duration::from_secs(120),
            engine.stream(
                "dd8255ecdc7ca55fb0bbf81323d87062db1f6d1c",
                None,
                None,
                &trackers,
            ),
        )
        .await
        .expect("metadata within 2 minutes")
        .unwrap();
        assert!(url.ends_with("/Big%20Buck%20Bunny.mp4"), "{url}");

        let response = tokio::time::timeout(
            Duration::from_secs(120),
            reqwest::Client::new()
                .get(&url)
                .header("Range", "bytes=0-65535")
                .send(),
        )
        .await
        .expect("first bytes within 2 minutes")
        .unwrap();
        assert_eq!(response.status(), 206);
        let bytes = response.bytes().await.unwrap();
        assert_eq!(bytes.len(), 65536);
        assert_eq!(&bytes[4..8], b"ftyp", "an MP4 starts with an ftyp box");

        let listed = engine.torrents();
        assert_eq!(listed.len(), 1);
        engine.remove(listed[0].id, true).await.unwrap();
        let _ = std::fs::remove_dir_all(dir);
    }
}
