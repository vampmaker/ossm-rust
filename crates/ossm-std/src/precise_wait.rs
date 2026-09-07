//! Platform waiter for the dedicated motor thread.
//!
//! Windows uses a high-resolution waitable timer plus an auto-reset event so
//! sub-millisecond wakes are not quantized to the 15.625 ms system tick.
//! Unix uses a Condvar (Linux: futex with ns timeout) plus a sticky flag.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[cfg(not(windows))]
use std::sync::Condvar;

#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    CancelWaitableTimer, CreateEventW, CreateWaitableTimerExW, SetEvent, SetWaitableTimer,
    WaitForMultipleObjects, WaitForSingleObject, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE,
    TIMER_ALL_ACCESS,
};

#[cfg(windows)]
const SPIN_MARGIN: Duration = Duration::from_micros(1_000);
#[cfg(all(unix, target_os = "macos"))]
const SPIN_MARGIN: Duration = Duration::from_micros(500);
#[cfg(all(unix, not(target_os = "macos")))]
const SPIN_MARGIN: Duration = Duration::from_micros(200);
#[cfg(not(any(windows, unix)))]
const SPIN_MARGIN: Duration = Duration::from_micros(1_000);

#[derive(Clone)]
pub struct Waker(Arc<Inner>);

pub struct Waiter(Arc<Inner>);

pub fn pair() -> (Waker, Waiter) {
    let inner = Arc::new(Inner::new());
    (Waker(inner.clone()), Waiter(inner))
}

impl Waker {
    pub fn notify(&self) {
        self.0.notify();
    }
}

impl Waiter {
    /// Block until `deadline` (None = forever) or a notify. Returns true if notified.
    pub fn wait_until(&mut self, deadline: Option<Instant>) -> bool {
        self.0.wait_until(deadline)
    }

    /// Sleep until `deadline`, returning early (true) on notify.
    ///
    /// Kernel-waits until `deadline - SPIN_MARGIN`, then spins so the last
    /// slice is not quantized by the OS timer.
    pub fn sleep_until(&mut self, deadline: Instant) -> bool {
        let spin_at = deadline.checked_sub(SPIN_MARGIN).unwrap_or(deadline);
        if spin_at > Instant::now() && self.wait_until(Some(spin_at)) {
            return true;
        }
        while Instant::now() < deadline {
            if self.0.poll_notified() {
                return true;
            }
            std::hint::spin_loop();
        }
        self.0.poll_notified()
    }

    pub fn describe(&self) -> &'static str {
        self.0.describe()
    }
}

/// Slot so a stream worker can start before the motor thread exists.
#[derive(Clone, Default)]
pub struct AckWaker(Arc<Mutex<Option<Waker>>>);

impl AckWaker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&self, waker: Waker) {
        *self.0.lock().unwrap() = Some(waker);
    }

    pub fn notify(&self) {
        if let Ok(g) = self.0.lock() {
            if let Some(w) = g.as_ref() {
                w.notify();
            }
        }
    }
}

#[cfg(not(windows))]
struct Inner {
    signaled: Mutex<bool>,
    cv: Condvar,
}

#[cfg(windows)]
struct SharedHandle(HANDLE);

#[cfg(windows)]
unsafe impl Send for SharedHandle {}
#[cfg(windows)]
unsafe impl Sync for SharedHandle {}

#[cfg(windows)]
struct Inner {
    timer: SharedHandle,
    event: SharedHandle,
    hires: bool,
}

#[cfg(not(windows))]
impl Inner {
    fn new() -> Self {
        Self {
            signaled: Mutex::new(false),
            cv: Condvar::new(),
        }
    }

    fn notify(&self) {
        let mut g = self.signaled.lock().unwrap();
        *g = true;
        self.cv.notify_one();
    }

    fn poll_notified(&self) -> bool {
        let mut g = self.signaled.lock().unwrap();
        if *g {
            *g = false;
            true
        } else {
            false
        }
    }

    fn wait_until(&self, deadline: Option<Instant>) -> bool {
        let mut g = self.signaled.lock().unwrap();
        loop {
            if *g {
                *g = false;
                return true;
            }
            match deadline {
                None => {
                    g = self.cv.wait(g).unwrap();
                }
                Some(d) => {
                    let now = Instant::now();
                    if now >= d {
                        return false;
                    }
                    let (gg, result) = self
                        .cv
                        .wait_timeout(g, d.saturating_duration_since(now))
                        .unwrap();
                    g = gg;
                    if result.timed_out() && !*g {
                        return false;
                    }
                }
            }
        }
    }

    fn describe(&self) -> &'static str {
        "condvar"
    }
}

#[cfg(windows)]
impl Inner {
    fn new() -> Self {
        unsafe {
            let event = CreateEventW(None, false, false, None).expect("CreateEventW");
            match CreateWaitableTimerExW(
                None,
                None,
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS.0,
            ) {
                Ok(timer) => Self {
                    timer: SharedHandle(timer),
                    event: SharedHandle(event),
                    hires: true,
                },
                Err(_) => {
                    begin_period_1ms();
                    let timer = CreateWaitableTimerExW(None, None, 0, TIMER_ALL_ACCESS.0)
                        .expect("CreateWaitableTimerExW");
                    Self {
                        timer: SharedHandle(timer),
                        event: SharedHandle(event),
                        hires: false,
                    }
                }
            }
        }
    }

    fn notify(&self) {
        unsafe {
            let _ = SetEvent(self.event.0);
        }
    }

    fn poll_notified(&self) -> bool {
        unsafe { WaitForSingleObject(self.event.0, 0) == WAIT_OBJECT_0 }
    }

    fn wait_until(&self, deadline: Option<Instant>) -> bool {
        unsafe {
            match deadline {
                None => WaitForSingleObject(self.event.0, INFINITE) == WAIT_OBJECT_0,
                Some(d) => {
                    let remaining = d.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return WaitForSingleObject(self.event.0, 0) == WAIT_OBJECT_0;
                    }
                    let hundred_ns = (remaining.as_nanos() / 100).max(1);
                    let due = -(hundred_ns as i64);
                    if SetWaitableTimer(self.timer.0, &due, 0, None, None, false).is_err() {
                        return WaitForSingleObject(
                            self.event.0,
                            remaining.as_millis().min(u32::MAX as u128) as u32,
                        ) == WAIT_OBJECT_0;
                    }
                    let handles = [self.event.0, self.timer.0];
                    let r = WaitForMultipleObjects(&handles, false, INFINITE);
                    let _ = CancelWaitableTimer(self.timer.0);
                    r.0 == WAIT_OBJECT_0.0
                }
            }
        }
    }

    fn describe(&self) -> &'static str {
        if self.hires {
            "waitable-timer-hires"
        } else {
            "waitable-timer"
        }
    }
}

#[cfg(windows)]
impl Drop for Inner {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.timer.0);
            let _ = CloseHandle(self.event.0);
        }
    }
}

#[cfg(windows)]
fn begin_period_1ms() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        windows::Win32::Media::timeBeginPeriod(1);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn sleep_until_3ms() {
        let (_w, mut waiter) = pair();
        let start = Instant::now();
        let deadline = start + Duration::from_millis(3);
        let notified = waiter.sleep_until(deadline);
        assert!(!notified);
        let elapsed = start.elapsed();
        assert!(
            elapsed >= Duration::from_millis(2),
            "too early: {elapsed:?}"
        );
        let late = elapsed.saturating_sub(Duration::from_millis(3));
        assert!(late < Duration::from_millis(10), "too late: {elapsed:?}");
    }

    #[test]
    fn notify_from_other_thread() {
        let (waker, mut waiter) = pair();
        let start = Instant::now();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(5));
            waker.notify();
        });
        let notified = waiter.wait_until(Some(Instant::now() + Duration::from_millis(200)));
        assert!(notified);
        assert!(
            start.elapsed() < Duration::from_millis(100),
            "notify was slow: {:?}",
            start.elapsed()
        );
    }

    #[test]
    fn notify_before_wait_is_not_lost() {
        let (waker, mut waiter) = pair();
        waker.notify();
        let notified = waiter.wait_until(Some(Instant::now() + Duration::from_millis(200)));
        assert!(notified);
    }
}
