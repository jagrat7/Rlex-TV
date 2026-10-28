//! Settings: library folders, stream sources, and where things end up.

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{ActiveTheme, Disableable, Icon, Sizable, StyledExt, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;

use super::common::{busy, card, eid, error_text, muted, page, page_title, pick_folders, section};
use crate::state::AppState;

pub struct SettingsView {
    state: Entity<AppState>,
    url_input: Entity<InputState>,
    adding: bool,
    add_error: Option<String>,
    _subscription: Subscription,
}

impl SettingsView {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        let url_input = cx
            .new(|cx| InputState::new(window, cx).placeholder("https://example.com/manifest.json"));
        let subscription = cx.subscribe_in(
            &url_input,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.add_source(window, cx),
                InputEvent::Change if this.add_error.take().is_some() => cx.notify(),
                _ => {}
            },
        );
        Self {
            state,
            url_input,
            adding: false,
            add_error: None,
            _subscription: subscription,
        }
    }

    fn add_source(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let url = self.url_input.read(cx).value().trim().to_string();
        if url.is_empty() || self.adding {
            return;
        }
        self.adding = true;
        self.add_error = None;
        cx.notify();
        let task = self
            .state
            .update(cx, |state, cx| state.install_addon(&url, cx));
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            this.update_in(cx, |this, window, cx| {
                this.adding = false;
                match result {
                    Ok(()) => this
                        .url_input
                        .update(cx, |input, cx| input.set_value("", window, cx)),
                    Err(err) => this.add_error = Some(format!("{err:#}")),
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn folders(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let (folders, scanning) = {
            let state = self.state.read(cx);
            (state.settings.library_folders.clone(), state.scanning)
        };
        let rows: Vec<Div> = folders
            .iter()
            .enumerate()
            .map(|(i, folder)| {
                let remove = folder.clone();
                h_flex()
                    .gap_3()
                    .items_center()
                    .px_4()
                    .py_2()
                    .when(i > 0, |this| this.border_t_1().border_color(theme.border))
                    .child(
                        Icon::new(IconName::Folder)
                            .size_4()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .truncate()
                            .child(folder.to_string_lossy().into_owned()),
                    )
                    .child(
                        Button::new(eid(format!("folder-remove-{i}")))
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state
                                    .update(cx, |state, cx| state.remove_folder(&remove, cx))
                            })),
                    )
            })
            .collect();

        let actions = h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("folders-add")
                    .primary()
                    .small()
                    .icon(IconName::FolderPlus)
                    .label("Add folder")
                    .on_click(cx.listener(|this, _, _, cx| pick_folders(this.state.clone(), cx))),
            )
            .child(
                Button::new("folders-rescan")
                    .outline()
                    .small()
                    .icon(IconName::RefreshCw)
                    .label("Rescan")
                    .disabled(scanning || folders.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |state, cx| state.rescan(false, cx))
                    })),
            )
            .child(
                Button::new("folders-retry")
                    .outline()
                    .small()
                    .icon(IconName::Sparkles)
                    .label("Retry unmatched")
                    .disabled(scanning || folders.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.state.update(cx, |state, cx| state.rescan(true, cx))
                    })),
            )
            .when(scanning, |this| this.child(busy("Scanning…", cx)));

        section(
            "Library folders",
            v_flex()
                .gap_3()
                .map(|this| {
                    if rows.is_empty() {
                        this.child(muted(
                            "No folders yet. Add one to start building your library.",
                            cx,
                        ))
                    } else {
                        this.child(card(cx).flex().flex_col().py_1().children(rows))
                    }
                })
                .child(actions),
        )
    }

    fn sources(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let addons = self.state.read(cx).sources().to_vec();
        let rows: Vec<Div> = addons
            .iter()
            .enumerate()
            .map(|(i, addon)| {
                let base_url = addon.base_url.clone();
                h_flex()
                    .gap_3()
                    .items_start()
                    .px_4()
                    .py_3()
                    .when(i > 0, |this| this.border_t_1().border_color(theme.border))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_medium()
                                            .truncate()
                                            .child(addon.name().to_string()),
                                    )
                                    .child(
                                        Tag::secondary()
                                            .small()
                                            .child(format!("v{}", addon.manifest.version)),
                                    ),
                            )
                            .when_some(addon.manifest.description.clone(), |this, text| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .line_clamp(2)
                                        .child(text),
                                )
                            })
                            .child(
                                div()
                                    .text_xs()
                                    .font_family(theme.mono_font_family.clone())
                                    .text_color(theme.muted_foreground)
                                    .truncate()
                                    .child(addon.base_url.clone()),
                            ),
                    )
                    .child(
                        Button::new(eid(format!("source-remove-{i}")))
                            .ghost()
                            .small()
                            .icon(IconName::Trash)
                            .label("Remove")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.state
                                    .update(cx, |state, cx| state.remove_addon(&base_url, cx))
                            })),
                    )
            })
            .collect();

        section(
            "Sources",
            v_flex()
                .gap_3()
                .child(muted(
                    "Rlex ships with no sources and only ever contacts the addresses you add here. Paste a Stremio addon's manifest URL to use it for catalogs and streams.",
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .max_w(px(720.))
                        .child(div().flex_1().child(Input::new(&self.url_input).disabled(self.adding)))
                        .child(
                            Button::new("source-add")
                                .primary()
                                .icon(IconName::Plus)
                                .label("Add")
                                .loading(self.adding)
                                .on_click(cx.listener(|this, _, window, cx| this.add_source(window, cx))),
                        ),
                )
                .when_some(self.add_error.clone(), |this, err| this.child(error_text(err, cx)))
                .map(|this| {
                    if rows.is_empty() {
                        this.child(muted("No sources added.", cx))
                    } else {
                        this.child(card(cx).flex().flex_col().children(rows))
                    }
                }),
        )
    }

    fn playback(&self, cx: &mut Context<Self>) -> Div {
        let theme = cx.theme();
        let download_dir = self.state.read(cx).settings.download_dir();
        v_flex()
            .gap_6()
            .child(section(
                "Playback",
                muted(
                    "Videos open in your desktop's default video app. Rlex follows the player and keeps your place automatically, so you can resume later.",
                    cx,
                ),
            ))
            .child(section(
                "Downloads",
                v_flex()
                    .gap_1()
                    .child(muted("Torrent streams are saved to this folder while they play.", cx))
                    .child(
                        div()
                            .text_sm()
                            .font_family(theme.mono_font_family.clone())
                            .child(download_dir.to_string_lossy().into_owned()),
                    ),
            ))
    }
}

impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        page("settings")
            .child(page_title("Settings"))
            .child(self.folders(cx))
            .child(self.sources(cx))
            .child(self.playback(cx))
    }
}
