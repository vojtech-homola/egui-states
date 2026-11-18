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
/// shortened to its maximum, about 24 days.
#[cfg(target_arch = "wasm32")]
pub(crate) fn sleep(duration: Duration) -> Sleep {
    let millis = i32::try_from(duration.as_millis()).unwrap_or(i32::MAX);
    Box::pin(async move {
        let promise = js_sys::Promise::new(&mut |resolve, _reject| {
            if let Some(window) = web_sys::window() {
                let _ =
                    window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, millis);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    })
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
