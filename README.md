<p align="center">
  <img src="assets/logo/rlex-256.png" width="128" height="128" alt="Rlex">
</p>

# Rlex

A lean media center for the desktop, written in Rust with [GPUI](https://gpui.rs) and
[GPUI Kit](https://github.com/longbridge/gpui-kit).

- **Your folders, like Plex.** Point Rlex at your movie and TV folders. It reads release-style
  and Plex-style names, matches them to titles and posters, and keeps track of what you've
  watched.
- **Stremio's catalogs and addons.** Browse and search with [Cinemeta](https://v3-cinemeta.strem.io/manifest.json)
  (Stremio's metadata service, no API key needed). Add any [Stremio addon](https://github.com/Stremio/stremio-addon-sdk/blob/master/docs/protocol.md)
  URL as a source.
- **Torrent streaming built in.** An in-process [librqbit](https://github.com/ikatson/rqbit)
  engine streams torrents from their first bytes, with a downloads page.
- **Plays in your own player.** Videos open in the desktop's default video app, and Rlex follows
  along: resume points, Continue watching, and watched marks all work.

Inspired by [Ventic](https://github.com/ventic/ventic).

> [!IMPORTANT]
> **Rlex ships with no sources and suggests none.** Cinemeta provides titles and artwork only.
> To find streams, you add the addon URLs yourself under *Settings → Sources*. What a source
> offers, and whether you may play it, is between you and whoever runs it.

## Running

Requirements (Linux, X11 or Wayland):

- Recent stable Rust via `rustup` (developed on 1.99)
- A Vulkan driver (Mesa is fine, including older Intel)
- `xdg-utils`, and a default video player. mpv is recommended; any MPRIS-capable player works.

```sh
cargo run --release
```

To get a launcher entry and the app icon in your dock (Wayland compositors find it through
the desktop entry's `app_id`):

```sh
cargo build --release && scripts/install-desktop.sh
```

This puts the binary in `~/.local/bin`, the desktop entry in `~/.local/share/applications`,
and the icons from `assets/logo/` into the hicolor theme.

## How playback and progress work

Rlex asks the desktop which app opens videos (`xdg-mime query default video/x-matroska`), reads
that app's desktop entry, and launches it with the file or stream URL.

| Default player | How Rlex follows it | Resume |
| --- | --- | --- |
| mpv | JSON IPC socket added to its own command line | `--start` |
| VLC, Celluloid, Haruna, Totem, … | MPRIS over D-Bus, matched by the URL it's playing | MPRIS `SetPosition` |

Positions are saved every few seconds. A video past 90% counts as watched. Torrents are served
to the player from `http://127.0.0.1:<port>/torrents/…`, read-only, on localhost only.

On macOS and Windows, videos open with the OS but progress isn't tracked yet.

## Your library

Rlex understands the usual layouts:

```
Movies/The.Matrix.1999.1080p.BluRay.x264.mkv
Movies/Inception (2010)/Inception.mkv
TV/Breaking Bad/Season 1/Breaking.Bad.S01E02.720p.mkv
TV/The Wire/Season 2/03 - Hot Shots.mkv
Anime/[Group] Show Name - 03 (1080p).mkv
```

A show can be spread over as many folders and naming styles as you like. Rlex sorts it out in
three steps:

1. **Group.** Files are clustered by title, ignoring punctuation and case, across every library
   folder. A year only splits a group when the files disagree on it (*Doctor Who* 1963 and
   2005). Inside `Show/Season N/`, the show folder names the show, as in Plex.
2. **Match.** Each group is looked up on Cinemeta once, with the best clues any of its files
   offer: a year from one file, the show folder's name, the file's own title. New files in a
   matched group need no lookup, and titles that turn out to be the same (an alias, a
   `Marvel's` prefix) merge under one match.
3. **Deduplicate.** A file reached twice (overlapping folders, symlinks, hardlinks) counts once.
   Separate copies of the same movie or episode are listed as **sources** for that video, best
   first: resolution, then source (Remux, BluRay, WEB-DL, WEBRip, HDTV, DVD), then size. Play
   uses the best one.

Files that match nothing still show up, under the name parsed from the file. Samples are skipped.

## Sources

A source is a Stremio addon base URL. Paste the `stremio://…` link or the `…/manifest.json`
address; Rlex keeps any configuration in the path. It asks each source for streams the
standard way:

```
GET {source}/stream/movie/{imdbId}.json
GET {source}/stream/series/{imdbId}:{season}:{episode}.json
```

Results from several sources are merged and deduplicated. A stream with an `infoHash` plays
through the torrent engine; a stream with a `url` (for example, from a debrid addon) plays
directly. Addons with catalogs also get rows on the home page.

## Where things are kept

| What | Where |
| --- | --- |
| Settings, sources, library index, watch history | `~/.local/share/rlex/*.json` |
| Torrent session (resumes on launch) | `~/.local/share/rlex/torrents/` |
| Downloads | `~/Downloads/Rlex/` |

## Code

| Module | Does |
| --- | --- |
| `src/state.rs` | The app state entity every view shares; runs slow work on Tokio |
| `src/addons.rs` | Stremio addon protocol client and types; Cinemeta |
| `src/library.rs` | Folder scanning, release-name parsing, Cinemeta matching |
| `src/torrent.rs` | librqbit session and the localhost streaming server |
| `src/player.rs` | Default-player launch; mpv IPC and MPRIS progress tracking |
| `src/watch.rs` | Watch history, resume points, Continue watching |
| `src/store.rs` | Data paths and atomic JSON files |
| `src/runtime.rs` | The Tokio runtime and how GPUI awaits it |
| `src/ui/` | GPUI views |

## Tests

```sh
cargo test                 # unit tests
cargo test -- --ignored    # live checks: Cinemeta, a CC-licensed torrent, mpv and MPRIS
```

The live player tests open video windows; run them with `--test-threads 1`.
