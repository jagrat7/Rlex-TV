//! Settings: library folders, stream sources, ratings, and where things end
//! up.

use rune::{Cursor, Cx, El, TextEdit, col, el, icon, row, text};
use rune_kit::TextInput;

use super::ratings::badges;
use super::theme::{space, t, ty};
use super::topbar::BAR_H;
use super::widgets::{
    Button, Size, Tone, Variant, busy, error_text, muted, page_title, ruled, section, tag,
};
use super::{App, Page, icons, scroller};
use crate::ratings::{KEY_URL, Ratings, built_in_key};

/// What's typed into Settings, and how the last try went.
pub struct Form {
    pub source_url: TextEdit,
    adding: bool,
    add_error: Option<String>,
    pub omdb_key: TextEdit,
    reveal_key: bool,
    testing_key: bool,
    /// What the last key test said.
    key_test: Option<Result<Ratings, String>>,
}

impl Default for Form {
    fn default() -> Self {
        Self {
            source_url: TextEdit::new(""),
            adding: false,
            add_error: None,
            omdb_key: TextEdit::new(""),
            reveal_key: false,
            testing_key: false,
            key_test: None,
        }
    }
}

impl App {
    fn add_source(&mut self) {
        let url = self.settings.source_url.text.trim().to_string();
        if url.is_empty() || self.settings.adding {
            return;
        }
        let fetch = match self.core.fetch_addon(&url) {
            Ok(fetch) => fetch,
            Err(err) => return self.settings.add_error = Some(format!("{err:#}")),
        };
        self.settings.adding = true;
        self.settings.add_error = None;
        self.bg.run(fetch, |s, addon, _| {
            s.settings.adding = false;
            let added = addon.and_then(|addon| {
                let name = addon.name().to_string();
                s.core.add_addon(addon).map(|()| name)
            });
            match added {
                Ok(name) => {
                    s.settings.source_url.clear();
                    s.toast_success(format!("Added {name}"));
                    s.sync_home();
                }
                Err(err) => s.settings.add_error = Some(format!("{err:#}")),
            }
        });
    }

    fn remove_source(&mut self, base_url: &str) {
        if let Err(err) = self.core.remove_addon(base_url) {
            self.toast_error(format!("{err:#}"));
        }
        self.sync_home();
    }

    /// Saves the key in the field (an empty field removes it), then tests it.
    fn save_key(&mut self) {
        let key = self.settings.omdb_key.text.trim().to_string();
        let empty = key.is_empty();
        if let Err(err) = self.core.set_omdb_key(Some(key)) {
            self.toast_error(format!("{err:#}"));
        }
        self.settings.key_test = None;
        if !empty {
            self.test_key();
        }
    }

    fn remove_key(&mut self) {
        if let Err(err) = self.core.set_omdb_key(None) {
            self.toast_error(format!("{err:#}"));
        }
        self.settings.omdb_key.clear();
        self.settings.key_test = None;
    }

    /// Tries the key in the field, or the one in use, with one request.
    fn test_key(&mut self) {
        if self.settings.testing_key {
            return;
        }
        let (key, request) = match self.core.omdb_test(&self.settings.omdb_key.text) {
            Ok(test) => test,
            Err(err) => return self.settings.key_test = Some(Err(format!("{err:#}"))),
        };
        self.settings.testing_key = true;
        self.settings.key_test = None;
        self.bg.run(request, move |s, result, _| {
            s.settings.testing_key = false;
            if result.is_ok() {
                s.core.omdb_key_works(&key);
            }
            s.settings.key_test = Some(result.map_err(|err| format!("{err:#}")));
        });
    }
}

pub fn page(s: &App, _: &mut Cx<App>) -> El {
    scroller(s, Page::Settings,
        col().key("settings")
            .w_full()
            .max_w(840.0)
            .px(space::PAGE)
            .pt(BAR_H + 28.0)
            .pb(64.0)
            .gap(space::SECTION)
            .child(page_title("Settings"))
            .child(folders(s))
            .child(sources(s))
            .child(ratings(s))
            .child(section(
                "Playback",
                muted("Videos open in your desktop's default video app. Rlex follows the player and keeps your place, so you can resume later."),
            ))
            .child(section(
                "Downloads",
                col().gap(6.0)
                    .child(muted("Torrent streams are saved to this folder while they play."))
                    .child(
                        text(s.core.settings.download_dir().to_string_lossy().into_owned())
                            .font(ty::MONO)
                            .color(t().text)
                            .selectable(true),
                    ),
            )),
    )
}

/// A row in a settings panel.
fn panel_row(i: usize) -> El {
    row()
        .key(format!("row-{i}"))
        .w_full()
        .gap(14.0)
        .px(16.0)
        .py(12.0)
        .center_y()
}

fn folders(s: &App) -> El {
    let th = t();
    let folders = &s.core.settings.library_folders;
    let scanning = s.core.scanning;
    let list = if folders.is_empty() {
        muted("No folders yet. Add one to start building your library.")
    } else {
        ruled(folders.iter().enumerate().map(|(i, folder)| {
            let remove = folder.clone();
            panel_row(i)
                .child(icon(&icons::FOLDER, 18.0).color(th.muted))
                .child(
                    text(folder.to_string_lossy().into_owned())
                        .font(ty::BODY)
                        .color(th.text)
                        .truncate_middle()
                        .grow(1.0)
                        .shrink(),
                )
                .child(
                    Button::icon(&icons::TRASH, "Remove this folder")
                        .variant(Variant::Ghost)
                        .size(Size::Small)
                        .on_press(move |s: &mut App, _| s.remove_folder(&remove))
                        .build(),
                )
        }))
    };
    let none = folders.is_empty();
    section(
        "Library folders",
        col().w_full().gap(14.0).child(list).child(
            row()
                .gap(8.0)
                .center_y()
                .child(
                    Button::new("Add folder")
                        .with_icon(&icons::FOLDER_PLUS)
                        .variant(Variant::Accent)
                        .size(Size::Small)
                        .on_press(|s: &mut App, tx| s.pick_folder(tx))
                        .build(),
                )
                .child(
                    Button::new("Rescan")
                        .with_icon(&icons::REFRESH)
                        .size(Size::Small)
                        .disabled(scanning || none)
                        .on_press(|s: &mut App, _| s.rescan(false))
                        .build(),
                )
                .child(
                    Button::new("Retry unmatched")
                        .with_icon(&icons::SPARKLES)
                        .size(Size::Small)
                        .disabled(scanning || none)
                        .help("Ask Cinemeta again about files it didn't recognise")
                        .on_press(|s: &mut App, _| s.rescan(true))
                        .build(),
                )
                .children(scanning.then(|| busy("Scanning…").ml(8.0))),
        ),
    )
}

fn sources(s: &App) -> El {
    let th = t();
    let form = &s.settings;
    let list = if s.core.sources().is_empty() {
        muted("No sources added.")
    } else {
        ruled(s.core.sources().iter().enumerate().map(|(i, addon)| {
            let base_url = addon.base_url.clone();
            panel_row(i)
                .items_start()
                .child(
                    col()
                        .gap(4.0)
                        .grow(1.0)
                        .shrink()
                        .child(
                            row()
                                .gap(8.0)
                                .center_y()
                                .child(
                                    text(addon.name().to_string())
                                        .font(ty::LABEL)
                                        .color(th.text)
                                        .truncate(),
                                )
                                .child(tag(format!("v{}", addon.manifest.version), Tone::Neutral)),
                        )
                        .children(
                            addon.manifest.description.clone().map(|about| {
                                text(about).font(ty::SMALL).color(th.muted).max_lines(2)
                            }),
                        )
                        .child(
                            text(addon.base_url.clone())
                                .font(ty::MONO)
                                .color(th.faint)
                                .truncate_middle(),
                        ),
                )
                .child(
                    Button::new("Remove")
                        .with_icon(&icons::TRASH)
                        .variant(Variant::Ghost)
                        .size(Size::Small)
                        .on_press(move |s: &mut App, _| s.remove_source(&base_url))
                        .build(),
                )
        }))
    };
    section(
        "Sources",
        col().w_full()
            .gap(14.0)
            .child(muted(
                "Rlex ships with no sources and only ever contacts the addresses you add here. Paste a Stremio addon's manifest URL to use it for catalogs and streams.",
            ))
            .child(
                row().w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        TextInput::new(&form.source_url)
                            .placeholder("https://example.com/manifest.json")
                            .label("Source address")
                            .width(None)
                            .height(34.0)
                            .disabled(form.adding)
                            .on_edit(|s: &mut App| &mut s.settings.source_url)
                            .on_change(|s: &mut App, _| s.settings.add_error = None)
                            .on_submit(|s: &mut App, _| s.add_source())
                            .build(&th.kit)
                            .grow(1.0),
                    )
                    .child(
                        Button::new("Add")
                            .with_icon(&icons::PLUS)
                            .variant(Variant::Accent)
                            .busy(form.adding)
                            .on_press(|s: &mut App, _| s.add_source())
                            .build(),
                    ),
            )
            .children(form.add_error.clone().map(error_text))
            .child(list),
    )
}

fn ratings(s: &App) -> El {
    let th = t();
    let form = &s.settings;
    let status = match &form.key_test {
        Some(Ok(ratings)) => Some(
            row()
                .gap(12.0)
                .center_y()
                .child(
                    text("The key works. The Shawshank Redemption:")
                        .font(ty::SMALL)
                        .color(th.success),
                )
                .children(badges(Some(ratings), None, true)),
        ),
        Some(Err(err)) => Some(error_text(err.clone())),
        None if form.testing_key => Some(busy("Testing the key…")),
        None => s
            .core
            .ratings_error()
            .map(|err| error_text(err.to_string())),
    };
    let reveal = form.reveal_key;
    section(
        "Ratings",
        col().w_full()
            .gap(14.0)
            .child(muted(if built_in_key().is_some() {
                "IMDb ratings come with Cinemeta, Rotten Tomatoes and Metacritic from Wikidata. OMDb fills the gaps (mostly TV shows) with a key built into Rlex. If it runs out, add your own free key."
            } else {
                "IMDb ratings come with Cinemeta, Rotten Tomatoes and Metacritic from Wikidata. Wikidata rarely has TV shows; add a free OMDb key to fill the gaps."
            }))
            .child(
                row().gap(6.0)
                    .center_y()
                    .child(text("Get a key at").font(ty::SMALL).color(th.muted))
                    .child(
                        row().key("omdb-link")
                            .gap(4.0)
                            .center_y()
                            .cursor(Cursor::Pointer)
                            .role("link")
                            .on_press_tx(|s: &mut App, tx| {
                                if let Err(err) = tx.open_url(KEY_URL) {
                                    s.toast_error(format!("Couldn't open the link: {err}"));
                                }
                            })
                            .child(text(KEY_URL).font(ty::SMALL_STRONG).color(th.accent_hover))
                            .child(icon(&icons::EXTERNAL, 13.0).color(th.accent_hover)),
                    ),
            )
            .child(
                row().w_full()
                    .gap(8.0)
                    .center_y()
                    .child(
                        TextInput::new(&form.omdb_key)
                            .placeholder("OMDb API key")
                            .label("OMDb API key")
                            .width(None)
                            .height(34.0)
                            .secure()
                            .revealed(reveal)
                            .trailing(
                                el().key("reveal")
                                    .size(24.0, 24.0)
                                    .radius(6.0)
                                    .center()
                                    .hover(th.hover)
                                    .cursor(Cursor::Pointer)
                                    .role("button")
                                    .label(if reveal { "Hide key" } else { "Show key" })
                                    .on_press(|s: &mut App| s.settings.reveal_key ^= true)
                                    .child(icon(if reveal { th.kit.glyph.eye_off } else { th.kit.glyph.eye }, 16.0).color(th.muted)),
                            )
                            .on_edit(|s: &mut App| &mut s.settings.omdb_key)
                            .on_change(|s: &mut App, _| s.settings.key_test = None)
                            .on_submit(|s: &mut App, _| s.save_key())
                            .build(&th.kit)
                            .grow(1.0),
                    )
                    .child(
                        Button::new("Save")
                            .with_icon(&icons::CHECK)
                            .variant(Variant::Accent)
                            .on_press(|s: &mut App, _| s.save_key())
                            .build(),
                    )
                    .child(Button::new("Test").busy(form.testing_key).on_press(|s: &mut App, _| s.test_key()).build())
                    .children(s.core.has_own_omdb_key().then(|| {
                        Button::new("Remove")
                            .with_icon(&icons::TRASH)
                            .variant(Variant::Ghost)
                            .on_press(|s: &mut App, _| s.remove_key())
                            .build()
                    })),
            )
            .children(status),
    )
}
