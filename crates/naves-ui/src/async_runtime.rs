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
