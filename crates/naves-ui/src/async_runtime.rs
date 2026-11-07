//! Minimal async-runtime adapter ported from `re_async` 0.38.1
//! (<https://github.com/rerun-io/rerun>, MIT OR Apache-2.0).
//!
//! Only the API surface used by `re_ui`'s `requested_object` is kept:
//! [`AsyncRuntimeHandle::spawn_future`] and the [`WasmNotSend`] bound.
//! Unlike upstream, no tokio/wasm-bindgen machinery is included: the caller
//! supplies the spawn function (e.g. a tokio `Handle`, bevy `TaskPool`, …),
//! or uses [`AsyncRuntimeHandle::new_thread_per_task`] on native.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

// ----------------------------------------------------------------------------

/// A future (or value) that is `Send` on native, but not necessarily on wasm.
///
/// Same pattern as `re_async`: on native everything must be `Send`, on wasm
/// (single-threaded) the bound is dropped.
#[cfg(not(target_arch = "wasm32"))]
pub trait WasmNotSend: Send {}
#[cfg(not(target_arch = "wasm32"))]
impl<T: Send> WasmNotSend for T {}

#[cfg(target_arch = "wasm32")]
pub trait WasmNotSend {}
#[cfg(target_arch = "wasm32")]
impl<T> WasmNotSend for T {}

// ----------------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
type BoxFuture = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;
#[cfg(target_arch = "wasm32")]
type BoxFuture = Pin<Box<dyn Future<Output = ()> + 'static>>;

/// A handle to an async runtime, used to spawn background fetches.
///
/// Minimal port of `re_async::AsyncRuntimeHandle`: a cheaply clonable wrapper
/// around a spawn closure.
#[derive(Clone)]
pub struct AsyncRuntimeHandle {
    spawn: Arc<dyn Fn(BoxFuture) + Send + Sync>,
}

impl AsyncRuntimeHandle {
    /// Wrap any spawn mechanism, e.g. a tokio `Handle`:
    /// `AsyncRuntimeHandle::new(move |fut| { tokio_handle.spawn(fut); })`.
    pub fn new(spawn: impl Fn(BoxFuture) + Send + Sync + 'static) -> Self {
        Self {
            spawn: Arc::new(spawn),
        }
    }

    /// Spawn a future onto the runtime.
    pub fn spawn_future(&self, future: impl Future<Output = ()> + WasmNotSend + 'static) {
        (self.spawn)(Box::pin(future));
    }

    /// A runtime that runs each future on its own `std::thread`, using a
    /// std-only `block_on`. Fine for occasional UI fetches.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new_thread_per_task() -> Self {
        Self::new(|fut| {
            std::thread::Builder::new()
                .name("naves_ui fetch".to_owned())
                .spawn(move || block_on(fut))
                .ok();
        })
    }
}

impl std::fmt::Debug for AsyncRuntimeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AsyncRuntimeHandle")
    }
}

/// A minimal std-only `block_on`: parks the thread until the future wakes it.
#[cfg(not(target_arch = "wasm32"))]
fn block_on<F: Future>(future: F) -> F::Output {
    use std::sync::Arc;
    use std::task::{Context, Poll, Wake};

    struct ThreadWaker(std::thread::Thread);

    impl Wake for ThreadWaker {
        fn wake(self: Arc<Self>) {
            self.0.unpark();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.0.unpark();
        }
    }

    let waker = Arc::new(ThreadWaker(std::thread::current())).into();
    let mut cx = Context::from_waker(&waker);
    let mut future = std::pin::pin!(future);

    loop {
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::park(),
        }
    }
}

// --- UiWaker (ported near-verbatim from re_async 0.42.0, MIT OR Apache-2.0) ---

use std::task::{Wake, Waker};

/// Wakes up a UI thread so that it repaints, e.g. when background work has produced a result.
///
/// Usually wraps `egui::Context::request_repaint`, but this crate does not depend on `egui`.
/// Use `crate::ContextExt::ui_waker` to create one from an `egui::Context`.
///
/// Cloning is cheap (a reference count increment).
#[derive(Clone, Debug)]
pub struct UiWaker(Waker);

impl UiWaker {
    /// A waker that does nothing.
    #[inline]
    #[must_use]
    pub fn noop() -> Self {
        Self(Waker::noop().clone())
    }

    /// Calls `wake` every time [`Self::wake`] is called.
    #[must_use]
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        struct FnWake<F>(F);

        impl<F: Fn() + Send + Sync + 'static> Wake for FnWake<F> {
            fn wake(self: Arc<Self>) {
                (self.0)();
            }

            fn wake_by_ref(self: &Arc<Self>) {
                (self.0)();
            }
        }

        Self(Waker::from(Arc::new(FnWake(wake))))
    }

    /// Wake up the UI.
    ///
    /// Safe to call from any thread.
    #[inline]
    pub fn wake(&self) {
        self.0.wake_by_ref();
    }
}

impl Default for UiWaker {
    #[inline]
    fn default() -> Self {
        Self::noop()
    }
}

impl From<Waker> for UiWaker {
    #[inline]
    fn from(waker: Waker) -> Self {
        Self(waker)
    }
}

impl From<UiWaker> for Waker {
    #[inline]
    fn from(waker: UiWaker) -> Self {
        waker.0
    }
}

#[cfg(test)]
mod ui_waker_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    #[test]
    fn wakes_every_clone() {
        let count = Arc::new(AtomicUsize::new(0));
        let waker = UiWaker::new({
            let count = count.clone();
            move || {
                count.fetch_add(1, Ordering::Relaxed);
            }
        });

        let clone = waker.clone();
        drop(waker);
        clone.wake();
        Waker::from(clone).wake();

        assert_eq!(count.load(Ordering::Relaxed), 2);
    }
}
