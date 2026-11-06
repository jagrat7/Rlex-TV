//! Background work: run a future on Tokio and hand its result to the app on
//! the UI thread, where it can change state and the view runs again.

use std::future::Future;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use rune::{AppHandle, Tx};

use super::App;
use crate::runtime;

#[derive(Clone)]
pub struct Bg {
    /// `None` in unit tests, which run without a shell.
    handle: Option<AppHandle<App>>,
    /// Work started and not yet handed back, so a headless render can wait
    /// for it and show what a person would see a moment after launch.
    pending: Arc<(Mutex<usize>, Condvar)>,
}

impl Bg {
    pub fn new(handle: AppHandle<App>) -> Self {
        Self {
            handle: Some(handle),
            pending: Default::default(),
        }
    }

    /// No shell to report to: results are dropped (unit tests, and the
    /// spec's fallback `init`, which a window never uses).
    pub fn detached() -> Self {
        Self {
            handle: None,
            pending: Default::default(),
        }
    }

    /// Runs `work` on Tokio, then `done` with its output on the UI thread.
    pub fn run<T: Send + 'static>(
        &self,
        work: impl Future<Output = T> + Send + 'static,
        done: impl FnOnce(&mut App, T, &mut Tx) + Send + 'static,
    ) {
        let Some(handle) = self.handle.clone() else {
            return;
        };
        let guard = self.begin();
        runtime::get().spawn(async move {
            let out = work.await;
            handle.update(move |app, tx| done(app, out, tx));
            drop(guard);
        });
    }

    /// Runs `work` on Tokio for its effect alone (an image landing in its
    /// `Pixels`), still counted for [`Self::settle`].
    pub fn spawn(&self, work: impl Future<Output = ()> + Send + 'static) {
        if self.handle.is_none() {
            return;
        }
        let guard = self.begin();
        runtime::get().spawn(async move {
            work.await;
            drop(guard);
        });
    }

    /// Changes state on the UI thread; the view runs again.
    pub fn update(&self, f: impl FnOnce(&mut App, &mut Tx) + Send + 'static) {
        if let Some(handle) = &self.handle {
            handle.update(f);
        }
    }

    /// Runs `poll` on the next frame, without changing anything itself.
    pub fn wake(&self) {
        if let Some(handle) = &self.handle {
            handle.wake();
        }
    }

    /// Waits, up to `timeout`, until no background work is outstanding.
    /// Headless renders only: a window never blocks on the network.
    pub fn settle(&self, timeout: Duration) {
        let (count, idle) = &*self.pending;
        let deadline = Instant::now() + timeout;
        let mut n = count.lock().unwrap();
        while *n > 0 {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            n = idle.wait_timeout(n, left).unwrap().0;
        }
    }

    fn begin(&self) -> Pending {
        *self.pending.0.lock().unwrap() += 1;
        Pending(self.pending.clone())
    }
}

/// One piece of outstanding work; done when dropped, even by a panic.
struct Pending(Arc<(Mutex<usize>, Condvar)>);

impl Drop for Pending {
    fn drop(&mut self) {
        let (count, idle) = &*self.0;
        let mut n = count.lock().unwrap();
        *n -= 1;
        if *n == 0 {
            idle.notify_all();
        }
    }
}
