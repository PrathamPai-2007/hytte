//! Minimal blocking driver for WinRT IAsync* (which expose IntoFuture).
//! No tokio; a timeout keeps workers from wedging.

use std::future::{Future as _, IntoFuture};
use std::pin::pin;
use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
use std::time::{Duration, Instant};

pub fn block_on<F: IntoFuture>(fut: F, timeout: Duration) -> Option<F::Output> {
    unsafe fn clone(_: *const ()) -> RawWaker {
        RawWaker::new(std::ptr::null(), &VTABLE)
    }
    unsafe fn noop(_: *const ()) {}
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    let waker = unsafe { Waker::from_raw(RawWaker::new(std::ptr::null(), &VTABLE)) };
    let mut cx = Context::from_waker(&waker);
    let mut fut = pin!(fut.into_future());
    let start = Instant::now();
    loop {
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => return Some(v),
            Poll::Pending => {
                if start.elapsed() > timeout {
                    return None;
                }
                std::thread::sleep(Duration::from_millis(4));
            }
        }
    }
}
