//! One Tokio runtime for networking and the torrent engine.
//!
//! GPUI runs its own executor. Futures spawned here resolve from any executor,
//! so GPUI tasks can await network and engine work without blocking the UI.

use std::future::Future;
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

/// Runs `fut` on the Tokio runtime and returns a future that resolves with its
/// result from any executor.
pub fn run<T, F>(fut: F) -> impl Future<Output = anyhow::Result<T>> + Send + 'static
where
    T: Send + 'static,
    F: Future<Output = anyhow::Result<T>> + Send + 'static,
{
    let handle = get().spawn(fut);
    async move { handle.await? }
}
