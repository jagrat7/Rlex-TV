//! Library: every title found in the user's folders, as a grid.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::{Sizable, h_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::common::await_player;
use super::common::{busy, eid, empty_state, page, page_title, pick_folders};
use super::poster::{PosterCard, poster_grid};
use super::quickplay::play_title;
use super::{DetailTarget, NavEvent};
use crate::library::LibraryTitle;
use crate::state::AppState;

pub struct LibraryView {
    state: Entity<AppState>,
    /// The title whose player is starting from a card's Play button.
    starting: Option<String>,
}

impl EventEmitter<NavEvent> for LibraryView {}

impl LibraryView {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Self {
            state,
            starting: None,
        }
    }

    fn play(&mut self, target: DetailTarget, window: &mut Window, cx: &mut Context<Self>) {
        if self.starting.is_some() {
            super::common::still_starting(window, cx);
            return;
        }
        self.starting = Some(target.id.clone());
        cx.notify();
        let task = play_title(&self.state, target, cx);
        await_player(task, window, cx, |this: &mut Self| this.starting = None);
    }

    fn add_folder_button(&self, id: &'static str, cx: &mut Context<Self>) -> Button {
        Button::new(id)
            .primary()
            .icon(IconName::FolderPlus)
            .label("Add folder")
            .on_click(cx.listener(|this, _, _, cx| pick_folders(this.state.clone(), cx)))
    }
}

/// The year for a movie; the episode count for a show.
fn card_subtitle(title: &LibraryTitle) -> Option<String> {
    if title.kind != "series" {
        return title.year.clone();
    }
    let episodes = title.videos().len();
    Some(match episodes {
        1 => "1 episode".to_string(),
        n => format!("{n} episodes"),
    })
}

impl Render for LibraryView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (titles, scanning) = {
            let state = self.state.read(cx);
            (state.library_titles(), state.scanning)
        };
        let cards: Vec<PosterCard> =
            titles
                .iter()
                .enumerate()
                .map(|(i, title)| {
                    let target = DetailTarget::from_title(title);
                    let play = target.clone();
                    PosterCard::new(eid(format!("library-{i}")), title.name.clone())
                        .poster(title.poster.clone())
                        .subtitle(card_subtitle(title))
                        .busy(self.starting.as_deref() == Some(title.meta_id.as_str()))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(NavEvent::OpenDetail(target.clone()))
                        }))
                        .on_play(cx.listener(move |this, _, window, cx| {
                            this.play(play.clone(), window, cx)
                        }))
                })
                .collect();

        page("library")
            .child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(page_title("My Library"))
                    .child(
                        h_flex()
                            .gap_4()
                            .items_center()
                            .when(scanning, |this| this.child(busy("Scanning folders…", cx)))
                            .child(self.add_folder_button("library-add", cx).small()),
                    ),
            )
            .map(|this| {
                if cards.is_empty() {
                    this.child(
                        empty_state(
                            IconName::FolderOpen,
                            if scanning { "Looking for videos…" } else { "Your library is empty" },
                            "Add a folder of movies or shows. Rlex matches the file names to titles and shows them here.",
                            cx,
                        )
                        .when(!scanning, |this| {
                            this.child(self.add_folder_button("library-add-empty", cx))
                        }),
                    )
                } else {
                    this.child(poster_grid(cards))
                }
            })
    }
}
