//! One Tokio runtime for networking, the torrent engine and image decoding.
//!
//! Rune owns the main thread. Work is spawned here, and its result comes back
//! to the UI through [`crate::ui::Bg`].

use std::sync::OnceLock;

use tokio::runtime::Runtime;

static RUNTIME: OnceLock<Runtime> = OnceLock::new();

pub fn get() -> &'static Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .thread_name("rlex-io")
            .enable_all()
            .build()
            .expect("failed to start the Tokio runtime")
    })
}
