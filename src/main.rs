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

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or(
        "info,librqbit=warn,librqbit_dht=warn,tracing=warn,zbus=warn,wgpu_hal=warn,naga=warn",
    ))
    .init();
    // Wayland compositors find the icon through the desktop entry with this id.
    rune::set_app_id("rlex");
    rune::prewarm_gpu();
    ui::run();
}
