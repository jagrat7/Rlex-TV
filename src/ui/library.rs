//! My Library: every title found in the user's folders, as a grid that fills
//! the window's width. It's a virtual list of grid lines, so a library of
//! thousands builds only the lines on screen. Also the library's actions:
//! adding folders and rescanning.

use std::borrow::Cow;
use std::path::PathBuf;
use std::rc::Rc;

use rune::{Cx, El, FileDialogOptions, GrowthSpec, Tx, col, row};

use super::poster::{CARD_W, Card};
use super::theme::space;
use super::topbar::BAR_H;
use super::widgets::{Button, Size, Variant, busy, empty_state, page_title};
use super::{Action, App, DetailTarget, Page, icons, scroller};
use crate::library::{self, Library, LibraryTitle};

const GAP: f32 = 18.0;

impl App {
    /// Asks for a folder and adds it to the library.
    pub fn pick_folder(&mut self, tx: &mut Tx) {
        let options = FileDialogOptions {
            title: "Add to library".into(),
            ..Default::default()
        };
        tx.open_folder(options, |s: &mut App, picked, _| match picked {
            Ok(Some(folder)) => s.add_folder(folder),
            Ok(None) => {}
            Err(err) => s.toast_error(format!("Couldn't open the folder picker: {err}")),
        });
    }

    pub fn add_folder(&mut self, folder: PathBuf) {
        if self.core.add_folder(folder) {
            self.save_settings();
            self.rescan(false);
        }
    }

    pub fn remove_folder(&mut self, folder: &PathBuf) {
        self.core.remove_folder(folder);
        self.save_settings();
        self.rescan(false);
    }

    pub fn save_settings(&mut self) {
        if let Err(err) = self.core.save_settings() {
            self.toast_error(format!("{err:#}"));
        }
    }

    /// Re-reads the library folders and matches new files to titles.
    /// `retry_unmatched` also asks Cinemeta again about files it didn't
    /// know before.
    pub fn rescan(&mut self, retry_unmatched: bool) {
        let Some((folders, previous)) = self.core.begin_scan(retry_unmatched) else {
            return;
        };
        // Walking the folders blocks, so it runs on Tokio's blocking pool.
        let scan = async move {
            tokio::task::spawn_blocking(move || {
                let files = library::scan(&folders, &previous);
                Library {
                    files,
                    misses: previous.misses,
                }
            })
            .await
        };
        self.bg.run(scan, |s, scanned, _| match scanned {
            Err(err) => {
                s.core.scanning = false;
                s.toast_error(format!("Library scan failed: {err:#}"));
            }
            Ok(library) => {
                // Show the files straight away; posters follow once matched.
                s.core.library = library.clone();
                let client = s.core.client.clone();
                let matched = async move {
                    let mut library = library;
                    library::match_files(&client, &mut library).await;
                    library
                };
                s.bg.run(matched, |s, library, _| {
                    s.core.scanning = false;
                    s.core.library = library;
                    s.core.save_library();
                });
            }
        });
    }
}

pub fn page(s: &App, cx: &mut Cx<App>) -> El {
    let titles: Rc<[LibraryTitle]> = s.core.library.titles().into();
    if titles.is_empty() {
        return scroller(s, Page::Library).child(
            col().key("library")
                .w_full()
                .px(space::PAGE)
                .pt(BAR_H + 28.0)
                .child(header(s))
                .child(
                    empty_state(
                        &icons::FOLDER_OPEN,
                        if s.core.scanning { "Looking for videos…" } else { "Your library is empty" },
                        "Add a folder of movies or shows. Rlex matches the file names to titles and shows them here.",
                    )
                    .children((!s.core.scanning).then(|| add_folder_button("library-add-empty", Size::Large))),
                ),
        );
    }

    // As many columns as fit, each card widened to share the spare room.
    let width = cx
        .layout_size(Page::Library.key())
        .map_or(1280.0, |[w, _]| w)
        - 2.0 * space::PAGE;
    let columns = (((width + GAP) / (CARD_W + GAP)).floor() as usize).max(1);
    let card_w = (width - GAP * (columns - 1) as f32) / columns as f32;
    let lines = titles.len().div_ceil(columns);
    let scale = cx.env().scale;

    let build = move |s: &App, i: usize| -> El {
        if i == 0 {
            return col()
                .w_full()
                .px(space::PAGE)
                .pt(BAR_H + 28.0)
                .pb(24.0)
                .child(header(s));
        }
        let start = (i - 1) * columns;
        let mut line = row()
            .w_full()
            .px(space::PAGE)
            .pb(28.0)
            .gap(GAP)
            .items_start();
        for (n, title) in titles[start..(start + columns).min(titles.len())]
            .iter()
            .enumerate()
        {
            line = line.child(title_card(s, start + n, title, card_w, scale));
        }
        line
    };
    scroller(s, Page::Library).child(
        col()
            .key("library-lines")
            .w_full()
            .growth(GrowthSpec::default())
            .virtual_rows(lines + 1, CARD_W * 1.5 + 80.0, line_key, build),
    )
}

/// The list's rows: the page header, then one per line of the grid.
fn line_key(_: &App, i: usize) -> Cow<'_, str> {
    match i {
        0 => "header".into(),
        i => format!("line-{i}").into(),
    }
}

fn header(s: &App) -> El {
    row()
        .w_full()
        .center_y()
        .justify_between()
        .child(page_title("My Library"))
        .child(
            row()
                .gap(16.0)
                .center_y()
                .children(s.core.scanning.then(|| busy("Scanning folders…")))
                .child(add_folder_button("library-add", Size::Medium)),
        )
}

fn add_folder_button(key: &str, size: Size) -> El {
    Button::new("Add folder")
        .with_icon(&icons::FOLDER_PLUS)
        .variant(Variant::Accent)
        .size(size)
        .key(key)
        .on_press(|s: &mut App, tx| s.pick_folder(tx))
        .build()
}

/// The year for a movie; the episode count for a show.
fn title_card(s: &App, i: usize, title: &LibraryTitle, width: f32, scale: f32) -> El {
    let subtitle = match title.kind.as_str() {
        "series" => match title.videos().len() {
            1 => "1 episode".to_string(),
            n => format!("{n} episodes"),
        },
        _ => title.year.clone().unwrap_or_default(),
    };
    let target = DetailTarget::from_title(title);
    Card::new(format!("{i}"), title.name.clone())
        .poster(title.poster.as_deref())
        .subtitle(Some(subtitle))
        .width(width)
        .busy(s.is_starting(&title.meta_id))
        .on_open(Action::Open(target.clone()))
        .on_play(Action::PlayTitle(target))
        .build(s, scale)
}
