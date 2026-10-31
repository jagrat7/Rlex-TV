//! Rlex: a lean media center. Your folders, like Plex; Stremio addons and
//! torrent streaming, like Stremio; played in your desktop's own video player.

mod addons;
mod library;
mod player;
mod ratings;
mod runtime;
mod state;
mod store;
mod torrent;
mod ui;
mod watch;

use std::sync::Arc;

use gpui_kit::AppContext as _;

fn main() {
    env_logger::Builder::from_env(
        env_logger::Env::default()
            .default_filter_or("info,librqbit=warn,librqbit_dht=warn,tracing=warn"),
    )
    .init();

    // Entered for the whole run so GPUI's HTTP client (posters) shares the runtime.
    let _runtime = runtime::get().enter();

    // The component library's icons come from its asset bundle.
    let app = gpui_kit::application().with_assets(gpui_kit::assets::AllAssets);
    app.run(|cx| {
        gpui_kit::init(cx);
        match reqwest_client::ReqwestClient::user_agent(concat!("Rlex/", env!("CARGO_PKG_VERSION")))
        {
            Ok(client) => cx.set_http_client(Arc::new(client)),
            Err(err) => log::error!("posters won't load: {err:#}"),
        }
        let state = cx.new(state::AppState::new);
        ui::open_main_window(state, cx);
    });
}
