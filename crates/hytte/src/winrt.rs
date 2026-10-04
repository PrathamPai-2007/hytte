//! Minimal blocking driver for WinRT IAsync* (which expose IntoFuture).
//! No tokio; a timeout keeps workers from wedging.

use std::future::{Future as _, IntoFuture};
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Poll, Wake, Waker};
use std::time::{Duration, Instant};

/// Wakes the blocked thread when the operation's `Completed` handler fires.
struct Unpark(std::thread::Thread);

impl Wake for Unpark {
    fn wake(self: Arc<Self>) {
        self.0.unpark();
    }
    fn wake_by_ref(self: &Arc<Self>) {
        self.0.unpark();
    }
}

pub fn block_on<F: IntoFuture>(fut: F, timeout: Duration) -> Option<F::Output> {
    let waker = Waker::from(Arc::new(Unpark(std::thread::current())));
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut.into_future());
    let deadline = Instant::now() + timeout;
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return Some(v),
            Poll::Pending => {
                let left = deadline.checked_duration_since(Instant::now())?;
                if left.is_zero() {
                    return None;
                }
                // Sleeps until completion instead of polling; spurious wake-ups just re-poll.
                std::thread::park_timeout(left);
            }
        }
    }
}
