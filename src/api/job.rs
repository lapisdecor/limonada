//! Off-thread request helper.
//!
//! egui is single-threaded and immediate-mode: a blocking HTTP call inside
//! `App::ui` would freeze the window for the duration of the request. So each
//! request runs on a short-lived worker thread and reports back over a channel
//! that the UI drains once per frame.
//!
//! This is deliberately not an async runtime. The API layer stays synchronous
//! (and therefore trivially testable), there is no executor to nest or drop,
//! and the only shared state is a `Sender` that moves into the thread.

use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::thread;

use super::error::ApiError;

/// The receiving end of an in-flight request.
pub struct Request<T> {
    rx: Receiver<Result<T, ApiError>>,
}

impl<T> Request<T> {
    /// Take the result if the worker has finished. `None` means still running.
    pub fn poll(&self) -> Option<Result<T, ApiError>> {
        match self.rx.try_recv() {
            Ok(value) => Some(value),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err(ApiError::Worker(
                "the background request thread stopped without reporting a result".into(),
            ))),
        }
    }
}

/// Run `work` on a worker thread, asking egui to repaint when it finishes.
///
/// `ctx.request_repaint()` is what makes the UI pick the result up promptly:
/// egui only redraws on input, timers or an explicit request, so without it a
/// completed request could sit unread until the user moved the mouse.
pub fn spawn<T, F>(ctx: egui::Context, work: F) -> Request<T>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    let (tx, rx) = mpsc::channel();

    let result = thread::Builder::new()
        .name("limonada-request".into())
        .spawn(move || {
            let outcome = work();
            // The receiver may be gone if the user navigated away; that is not
            // an error worth surfacing.
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });

    match result {
        Ok(_handle) => Request { rx },
        Err(err) => {
            // Could not start a thread: report it through the same channel so
            // the caller has a single code path.
            let (tx, rx) = mpsc::channel();
            let _ = tx.send(Err(ApiError::Worker(err.to_string())));
            Request { rx }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_a_finished_result() {
        let ctx = egui::Context::default();
        let request = spawn(ctx, || Ok::<_, ApiError>(7));
        // The worker may need a moment; poll briefly rather than sleeping a
        // fixed amount.
        let mut result = None;
        for _ in 0..200 {
            if let Some(value) = request.poll() {
                result = Some(value);
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        assert_eq!(result.expect("should finish").unwrap(), 7);
    }

    #[test]
    fn reports_none_while_still_running() {
        let ctx = egui::Context::default();
        let request = spawn(ctx, || {
            thread::sleep(std::time::Duration::from_millis(120));
            Ok::<_, ApiError>(1)
        });
        assert!(request.poll().is_none(), "should still be running");
    }

    #[test]
    fn propagates_the_worker_error() {
        let ctx = egui::Context::default();
        let request = spawn(ctx, || Err::<(), _>(ApiError::Lemmy("nope".into())));
        let mut result = None;
        for _ in 0..200 {
            if let Some(value) = request.poll() {
                result = Some(value);
                break;
            }
            thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(matches!(result, Some(Err(ApiError::Lemmy(_)))));
    }
}
