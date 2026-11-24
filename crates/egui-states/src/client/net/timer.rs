//! A timer for the connection task, native or in the browser.

use std::pin::Pin;
use std::time::Duration;

/// Completes once its duration has passed.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) type Sleep = Pin<Box<tokio::time::Sleep>>;

/// Completes once its duration has passed.
#[cfg(target_arch = "wasm32")]
pub(crate) type Sleep = Pin<Box<dyn std::future::Future<Output = ()>>>;

/// Returns a timer that completes after `duration`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn sleep(duration: Duration) -> Sleep {
    // Tokio caps durations too large for its clock.
    Box::pin(tokio::time::sleep(duration))
}

/// Returns a timer that completes after `duration`.
///
/// It uses the window's `setTimeout`, so it never completes where there is no
/// window, such as in a worker. Durations beyond what `setTimeout` takes are
/// shortened to its maximum, about 24 days. The timer runs from this call,
/// and dropping it cancels the timeout.
#[cfg(target_arch = "wasm32")]
pub(crate) fn sleep(duration: Duration) -> Sleep {
    Box::pin(Timeout::new(duration))
}

/// A `setTimeout` of the window that is cleared when dropped.
///
/// The connection task drops most of its timers before they are due: one per
/// received frame while a heartbeat is watched. Left alone, each would stay
/// in the browser until its full duration had passed. A promise is not used
/// for the same reason: one that never resolves keeps its callbacks forever.
#[cfg(target_arch = "wasm32")]
struct Timeout {
    /// The handle `setTimeout` returned; `None` where there is no window.
    handle: Option<i32>,
    fired: std::rc::Rc<Fired>,
    /// What the timeout calls; it must live until the timeout ran or is
    /// cleared.
    _callback: web_sys::wasm_bindgen::closure::Closure<dyn FnMut()>,
}

/// Whether a [`Timeout`] ran, and who waits for it.
#[cfg(target_arch = "wasm32")]
#[derive(Default)]
struct Fired {
    done: std::cell::Cell<bool>,
    waker: std::cell::RefCell<Option<std::task::Waker>>,
}

#[cfg(target_arch = "wasm32")]
impl Timeout {
    fn new(duration: Duration) -> Self {
        use web_sys::wasm_bindgen::JsCast;
        use web_sys::wasm_bindgen::closure::Closure;

        let millis = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
        let fired = std::rc::Rc::new(Fired::default());
        let callback = Closure::<dyn FnMut()>::new({
            let fired = fired.clone();
            move || {
                fired.done.set(true);
                let waker = fired.waker.borrow_mut().take();
                if let Some(waker) = waker {
                    waker.wake();
                }
            }
        });
        let handle = web_sys::window().and_then(|window| {
            window
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    callback.as_ref().unchecked_ref(),
                    millis,
                )
                .ok()
        });
        Self {
            handle,
            fired,
            _callback: callback,
        }
    }
}

#[cfg(target_arch = "wasm32")]
impl std::future::Future for Timeout {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut std::task::Context<'_>) -> std::task::Poll<()> {
        if self.fired.done.get() {
            return std::task::Poll::Ready(());
        }
        *self.fired.waker.borrow_mut() = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

#[cfg(target_arch = "wasm32")]
impl Drop for Timeout {
    fn drop(&mut self) {
        // The callback is dropped with this timer, so the timeout must not
        // call it anymore. Clearing one that already ran does nothing.
        if let (Some(handle), Some(window)) = (self.handle, web_sys::window()) {
            window.clear_timeout_with_handle(handle);
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn sleep_completes_after_its_duration() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        runtime.block_on(async {
            let start = tokio::time::Instant::now();
            sleep(Duration::from_millis(20)).await;
            assert!(start.elapsed() >= Duration::from_millis(20));
            // A huge duration builds a timer instead of panicking.
            drop(sleep(Duration::MAX));
        });
    }
}
