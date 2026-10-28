//! Hands playback to the desktop's default video player and follows along to
//! record how far you got.
//!
//! The player is whatever the desktop opens videos with (`xdg-mime`). How
//! progress comes back depends on the player:
//!
//! - **mpv** (the usual Linux default) is launched from its own desktop entry
//!   with a JSON IPC socket added, then asked for `time-pos` directly.
//!   `--start` resumes.
//! - **Anything else** is followed over MPRIS, the D-Bus interface VLC,
//!   Celluloid, Haruna, Totem and most other players publish. Resume uses
//!   MPRIS `SetPosition` once the video has loaded.
//!
//! A player with neither still plays; it just reports nothing.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context as _, bail};
use tokio::sync::mpsc;

/// How often a position is passed on while playing.
const REPORT_EVERY: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub enum Progress {
    /// Seconds into the video, and its length (0 when unknown).
    Position { position: f64, duration: f64 },
    /// The player closed or moved on to something else.
    Ended,
}

/// What to play.
#[derive(Clone, Debug)]
pub struct Launch {
    /// A local path, or an `http(s)` URL.
    pub target: String,
    /// Window title for players that take one.
    pub title: String,
    /// Seconds to resume from.
    pub start: Option<f64>,
}

/// Opens `launch` in the default player. Progress reports arrive on the
/// returned channel until [`Progress::Ended`]. Must run on the Tokio runtime.
pub async fn play(launch: Launch) -> anyhow::Result<mpsc::UnboundedReceiver<Progress>> {
    let (tx, rx) = mpsc::unbounded_channel();
    platform::play(launch, tx).await?;
    Ok(rx)
}

fn is_url(target: &str) -> bool {
    target.starts_with("http://") || target.starts_with("https://")
}

/// The MIME type the desktop's default-app lookup should use for `target`.
fn mime_for(target: &str) -> &'static str {
    let path = target.split(['?', '#']).next().unwrap_or(target);
    let ext = Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "avi" => "video/x-msvideo",
        "mov" => "video/quicktime",
        "wmv" => "video/x-ms-wmv",
        "ts" | "m2ts" => "video/mp2t",
        "mpg" | "mpeg" => "video/mpeg",
        "flv" => "video/x-flv",
        "ogv" => "video/ogg",
        _ => "video/x-matroska",
    }
}

/// Splits a desktop entry `Exec=` value into arguments, substituting the
/// target for its field codes.
/// <https://specifications.freedesktop.org/desktop-entry-spec/latest/exec-variables.html>
fn expand_exec(exec: &str, target: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_arg = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                in_arg = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if in_arg {
                    args.push(std::mem::take(&mut current));
                    in_arg = false;
                }
            }
            c => {
                current.push(c);
                in_arg = true;
            }
        }
    }
    if in_arg {
        args.push(current);
    }

    let mut has_target = false;
    let mut expanded = Vec::new();
    for arg in args {
        match arg.as_str() {
            "%f" | "%F" | "%u" | "%U" => {
                expanded.push(target.to_string());
                has_target = true;
            }
            "%i" | "%c" | "%k" => {}
            _ => {
                let arg = arg.replace("%%", "\u{0}");
                let arg = ["%f", "%F", "%u", "%U"]
                    .iter()
                    .fold(arg, |arg, code| {
                        if arg.contains(code) {
                            has_target = true;
                        }
                        arg.replace(code, target)
                    })
                    .replace("%i", "")
                    .replace("%c", "")
                    .replace("%k", "")
                    .replace('\u{0}', "%");
                expanded.push(arg);
            }
        }
    }
    if !has_target {
        expanded.push(target.to_string());
    }
    expanded
}

/// The value of `key` in a desktop entry's `[Desktop Entry]` group.
fn desktop_entry_value(contents: &str, key: &str) -> Option<String> {
    let mut in_main_group = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_main_group = line == "[Desktop Entry]";
        } else if in_main_group
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

fn program_name(args: &[String]) -> String {
    args.first()
        .map(|p| {
            Path::new(p)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
mod platform {
    use std::collections::HashMap;

    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;
    use tokio::process::Command;
    use tokio::time::{Instant, sleep, timeout};
    use zbus::zvariant::OwnedValue;

    use super::*;

    pub async fn play(launch: Launch, tx: mpsc::UnboundedSender<Progress>) -> anyhow::Result<()> {
        let target = if is_url(&launch.target) {
            launch.target.clone()
        } else {
            let path = Path::new(&launch.target);
            if !path.exists() {
                bail!("{} no longer exists", path.display());
            }
            launch.target.clone()
        };

        let Some(mut args) = default_player_command(&target).await else {
            // No desktop entry to read: let xdg-open decide, and follow over MPRIS.
            spawn(&["xdg-open".to_string(), target.clone()])?;
            tokio::spawn(follow_mpris(target, launch.start, tx));
            return Ok(());
        };

        if program_name(&args) == "mpv" {
            let socket = crate::store::runtime_dir().join(format!(
                "rlex-mpv-{}-{}.sock",
                std::process::id(),
                crate::store::now()
            ));
            let mut options = vec![
                format!("--input-ipc-server={}", socket.display()),
                format!("--force-media-title={}", launch.title),
            ];
            if let Some(start) = launch.start {
                options.push(format!("--start={start:.0}"));
            }
            // Options go right after the program, before any `--`.
            args.splice(1..1, options);
            let child = spawn(&args)?;
            tokio::spawn(follow_mpv(socket, child, tx));
        } else {
            spawn(&args)?;
            tokio::spawn(follow_mpris(target, launch.start, tx));
        }
        Ok(())
    }

    fn spawn(args: &[String]) -> anyhow::Result<tokio::process::Child> {
        let (program, rest) = args.split_first().context("empty player command")?;
        log::info!("playing with {args:?}");
        Command::new(program)
            .args(rest)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .spawn()
            .with_context(|| format!("couldn't start {program}"))
    }

    /// The command the default video app's desktop entry runs for `target`.
    async fn default_player_command(target: &str) -> Option<Vec<String>> {
        let output = Command::new("xdg-mime")
            .args(["query", "default", mime_for(target)])
            .output()
            .await
            .ok()?;
        let desktop_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if desktop_id.is_empty() {
            return None;
        }
        let contents =
            find_desktop_file(&desktop_id).and_then(|path| std::fs::read_to_string(path).ok())?;
        let exec = desktop_entry_value(&contents, "Exec")?;
        Some(expand_exec(&exec, target))
    }

    fn find_desktop_file(desktop_id: &str) -> Option<PathBuf> {
        let data_home = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| dirs::home_dir().map(|h| h.join(".local/share")));
        let data_dirs =
            std::env::var("XDG_DATA_DIRS").unwrap_or_else(|_| "/usr/local/share:/usr/share".into());
        // `kde-foo.desktop` may live at `kde/foo.desktop`.
        let nested = desktop_id.replacen('-', "/", 1);
        data_home
            .into_iter()
            .chain(
                data_dirs
                    .split(':')
                    .filter(|d| !d.is_empty())
                    .map(PathBuf::from),
            )
            .flat_map(|dir| {
                let apps = dir.join("applications");
                [apps.join(desktop_id), apps.join(&nested)]
            })
            .find(|path| path.is_file())
    }

    /// Follows mpv over its JSON IPC socket until it quits.
    async fn follow_mpv(
        socket: PathBuf,
        mut child: tokio::process::Child,
        tx: mpsc::UnboundedSender<Progress>,
    ) {
        let result = async {
            let deadline = Instant::now() + Duration::from_secs(20);
            let stream = loop {
                match UnixStream::connect(&socket).await {
                    Ok(stream) => break stream,
                    Err(_) if Instant::now() < deadline => sleep(Duration::from_millis(250)).await,
                    Err(err) => return Err(anyhow::Error::from(err).context("mpv IPC")),
                }
            };
            let (reader, mut writer) = stream.into_split();
            writer
                .write_all(
                    concat!(
                        r#"{"command":["observe_property",1,"time-pos"]}"#,
                        "\n",
                        r#"{"command":["observe_property",2,"duration"]}"#,
                        "\n"
                    )
                    .as_bytes(),
                )
                .await?;

            let mut lines = BufReader::new(reader).lines();
            // No report until mpv has a real position: `duration` can arrive
            // first, and reporting 0 would wipe the resume point.
            let (mut position, mut duration) = (None, 0.0);
            let mut last_report = Instant::now() - REPORT_EVERY;
            while let Some(line) = lines.next_line().await? {
                let Ok(event) = serde_json::from_str::<serde_json::Value>(&line) else {
                    continue;
                };
                if event["event"] != "property-change" {
                    continue;
                }
                let Some(value) = event["data"].as_f64() else {
                    continue;
                };
                match event["name"].as_str() {
                    Some("time-pos") => position = Some(value),
                    Some("duration") => duration = value,
                    _ => continue,
                }
                if let Some(position) = position
                    && last_report.elapsed() >= REPORT_EVERY
                {
                    last_report = Instant::now();
                    let _ = tx.send(Progress::Position { position, duration });
                }
            }
            // The socket closes when mpv quits; send where it stopped.
            if let Some(position) = position {
                let _ = tx.send(Progress::Position { position, duration });
            }
            anyhow::Ok(())
        }
        .await;
        if let Err(err) = result {
            log::warn!("lost track of mpv: {err:#}");
        }
        let _ = tx.send(Progress::Ended);
        let _ = child.wait().await;
        let _ = std::fs::remove_file(&socket);
    }

    /// Finds the MPRIS player showing `target` and follows it until it closes
    /// or moves on to another video.
    pub(super) async fn follow_mpris(
        target: String,
        start: Option<f64>,
        tx: mpsc::UnboundedSender<Progress>,
    ) {
        let result = async {
            let conn = zbus::Connection::session().await?;
            let deadline = Instant::now() + Duration::from_secs(60);
            let player = loop {
                if let Some(player) = find_player(&conn, &target).await {
                    break player;
                }
                if Instant::now() > deadline {
                    log::info!("no MPRIS player showed {target}; progress won't be tracked");
                    return anyhow::Ok(());
                }
                sleep(Duration::from_secs(1)).await;
            };

            if let Some(start) = start {
                // Players accept a seek only once the video has loaded.
                sleep(Duration::from_secs(1)).await;
                if let Some(track) = metadata(&conn, &player).await.and_then(|m| track_id(&m)) {
                    let micros = (start * 1_000_000.0) as i64;
                    let _ = timeout(
                        Duration::from_secs(3),
                        conn.call_method(
                            Some(player.as_str()),
                            "/org/mpris/MediaPlayer2",
                            Some("org.mpris.MediaPlayer2.Player"),
                            "SetPosition",
                            &(track, micros),
                        ),
                    )
                    .await;
                }
            }

            let (mut position, mut duration) = (0.0, 0.0);
            loop {
                let Some(meta) = metadata(&conn, &player).await else {
                    break; // the player quit
                };
                if !meta_url(&meta).is_some_and(|url| same_media(&url, &target)) {
                    break; // it moved on to something else
                }
                duration = length(&meta).unwrap_or(duration);
                if let Some(p) = property(&conn, &player, "Position")
                    .await
                    .and_then(|v| i64::try_from(v).ok())
                {
                    position = p as f64 / 1_000_000.0;
                }
                // A player that hasn't started (or ignored the resume seek)
                // reports 0; don't let that wipe the resume point.
                if position > 0.0 {
                    let _ = tx.send(Progress::Position { position, duration });
                }
                sleep(REPORT_EVERY).await;
            }
            if position > 0.0 {
                let _ = tx.send(Progress::Position { position, duration });
            }
            anyhow::Ok(())
        }
        .await;
        if let Err(err) = result {
            log::warn!("MPRIS tracking failed: {err:#}");
        }
        let _ = tx.send(Progress::Ended);
    }

    async fn find_player(conn: &zbus::Connection, target: &str) -> Option<String> {
        let reply = conn
            .call_method(
                Some("org.freedesktop.DBus"),
                "/org/freedesktop/DBus",
                Some("org.freedesktop.DBus"),
                "ListNames",
                &(),
            )
            .await
            .ok()?;
        let names: Vec<String> = reply.body().deserialize().ok()?;
        for name in names
            .into_iter()
            .filter(|n| n.starts_with("org.mpris.MediaPlayer2."))
        {
            let showing = metadata(conn, &name)
                .await
                .and_then(|m| meta_url(&m))
                .is_some_and(|url| same_media(&url, target));
            if showing {
                return Some(name);
            }
        }
        None
    }

    async fn property(conn: &zbus::Connection, player: &str, name: &str) -> Option<OwnedValue> {
        let reply = timeout(
            Duration::from_secs(2),
            conn.call_method(
                Some(player),
                "/org/mpris/MediaPlayer2",
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &("org.mpris.MediaPlayer2.Player", name),
            ),
        )
        .await
        .ok()?
        .ok()?;
        reply.body().deserialize::<OwnedValue>().ok()
    }

    async fn metadata(
        conn: &zbus::Connection,
        player: &str,
    ) -> Option<HashMap<String, OwnedValue>> {
        HashMap::try_from(property(conn, player, "Metadata").await?).ok()
    }

    fn meta_url(meta: &HashMap<String, OwnedValue>) -> Option<String> {
        String::try_from(meta.get("xesam:url")?.try_clone().ok()?).ok()
    }

    fn track_id(meta: &HashMap<String, OwnedValue>) -> Option<zbus::zvariant::OwnedObjectPath> {
        let value = meta.get("mpris:trackid")?.try_clone().ok()?;
        zbus::zvariant::OwnedObjectPath::try_from(value).ok()
    }

    /// Seconds. Players send `mpris:length` as either a signed or unsigned integer.
    fn length(meta: &HashMap<String, OwnedValue>) -> Option<f64> {
        let value = meta.get("mpris:length")?;
        let micros = i64::try_from(value.try_clone().ok()?).ok().or_else(|| {
            u64::try_from(value.try_clone().ok()?)
                .ok()
                .map(|v| v as i64)
        })?;
        (micros > 0).then(|| micros as f64 / 1_000_000.0)
    }
}

#[cfg(not(target_os = "linux"))]
mod platform {
    use super::*;

    /// Opens the target with the OS; progress isn't tracked here yet.
    pub async fn play(launch: Launch, tx: mpsc::UnboundedSender<Progress>) -> anyhow::Result<()> {
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(target_os = "windows")]
        let mut command = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        };
        command
            .arg(&launch.target)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("couldn't open the video")?;
        let _ = tx.send(Progress::Ended);
        Ok(())
    }
}

/// Whether an MPRIS `xesam:url` is the file or URL that was launched.
fn same_media(reported: &str, target: &str) -> bool {
    let decode = |s: &str| {
        urlencoding::decode(s)
            .map(|d| d.into_owned())
            .unwrap_or_else(|_| s.to_string())
    };
    let reported = decode(reported.strip_prefix("file://").unwrap_or(reported));
    let target = decode(target.strip_prefix("file://").unwrap_or(target));
    if reported == target {
        return true;
    }
    // Players sometimes normalize URLs; the last path segment survives that.
    let last = |s: &str| {
        s.trim_end_matches('/')
            .rsplit('/')
            .next()
            .map(str::to_string)
    };
    is_url(&target) && last(&reported).is_some_and(|r| !r.is_empty() && Some(r) == last(&target))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_desktop_exec_lines() {
        assert_eq!(
            expand_exec("mpv --player-operation-mode=pseudo-gui -- %U", "/v/a b.mkv"),
            [
                "mpv",
                "--player-operation-mode=pseudo-gui",
                "--",
                "/v/a b.mkv"
            ]
        );
        assert_eq!(
            expand_exec(r#""/opt/My Player/bin" --x %i %f"#, "u"),
            ["/opt/My Player/bin", "--x", "u"]
        );
        assert_eq!(expand_exec("vlc", "u"), ["vlc", "u"]);
        assert_eq!(
            expand_exec("p --name=100%% --open=%u", "u"),
            ["p", "--name=100%", "--open=u"]
        );
    }

    #[test]
    fn reads_desktop_entries() {
        let entry =
            "[Desktop Entry]\nName=mpv\nExec=mpv -- %U\n[Desktop Action new]\nExec=mpv --new\n";
        assert_eq!(
            desktop_entry_value(entry, "Exec").as_deref(),
            Some("mpv -- %U")
        );
        assert_eq!(program_name(&expand_exec("/usr/bin/mpv %U", "x")), "mpv");
    }

    #[test]
    fn matches_mpris_urls() {
        assert!(same_media("file:///v/a%20b.mkv", "/v/a b.mkv"));
        assert!(same_media(
            "http://127.0.0.1:1/torrents/0/stream/2/Ep%201.mkv",
            "http://127.0.0.1:1/torrents/0/stream/2/Ep%201.mkv"
        ));
        assert!(!same_media("file:///v/other.mkv", "/v/a.mkv"));
        assert_eq!(mime_for("http://x/stream/0/Movie.MP4"), "video/mp4");
    }
}

/// Plays a generated 12-second clip in real players:
/// `cargo test -- --ignored live_ --test-threads 1`. Opens video windows.
#[cfg(all(test, target_os = "linux"))]
mod live_tests {
    use super::*;

    fn clip() -> PathBuf {
        let path = std::env::temp_dir().join("rlex-test-clip.mkv");
        if !path.exists() {
            let status = std::process::Command::new("ffmpeg")
                .args(["-loglevel", "error", "-f", "lavfi"])
                .args(["-i", "testsrc=duration=12:size=320x240:rate=25"])
                .args(["-c:v", "libx264", "-y"])
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
        }
        path
    }

    /// Positions reported, after checking the stream ends with `Ended`.
    async fn positions(mut rx: mpsc::UnboundedReceiver<Progress>) -> Vec<(f64, f64)> {
        let mut positions = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(60), rx.recv()).await {
                Ok(Some(Progress::Position { position, duration })) => {
                    positions.push((position, duration))
                }
                Ok(Some(Progress::Ended)) => return positions,
                Ok(None) => panic!("channel closed without Ended"),
                Err(_) => panic!("no progress for 60s"),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "opens the default video player"]
    async fn live_default_player() {
        let rx = play(Launch {
            target: clip().display().to_string(),
            title: "Rlex test clip".into(),
            start: Some(4.0),
        })
        .await
        .unwrap();
        let positions = positions(rx).await;
        eprintln!("default player reported {positions:?}");
        let (first, _) = positions[0];
        let (last, duration) = *positions.last().unwrap();
        assert!(first >= 3.9, "should resume at 4s, started at {first}");
        assert!((duration - 12.0).abs() < 0.5, "duration {duration}");
        assert!(last > 11.0, "should play to the end, stopped at {last}");
    }

    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "opens mpv and follows it over MPRIS"]
    async fn live_mpris() {
        let clip = clip();
        let mut child = tokio::process::Command::new("mpv")
            .arg("--really-quiet")
            .arg(&clip)
            .spawn()
            .unwrap();
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(platform::follow_mpris(
            clip.display().to_string(),
            Some(5.0),
            tx,
        ));
        let positions = positions(rx).await;
        eprintln!("MPRIS reported {positions:?}");
        let _ = child.wait().await;
        let (first, _) = positions[0];
        let (last, duration) = *positions.last().unwrap();
        assert!(first >= 4.5, "should resume at 5s, started at {first}");
        assert!((duration - 12.0).abs() < 0.5, "duration {duration}");
        assert!(last > 9.0, "stopped at {last}");
    }
}
