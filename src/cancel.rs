use anyhow::Result;
use std::{
    fmt,
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

#[derive(Debug)]
pub struct Cancelled;
impl fmt::Display for Cancelled {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Stopped")
    }
}
impl std::error::Error for Cancelled {}
pub fn check(stop: &AtomicBool) -> Result<()> {
    if stop.load(Ordering::Acquire) {
        Err(Cancelled.into())
    } else {
        Ok(())
    }
}
pub fn pause(stop: &AtomicBool, duration: Duration) -> Result<()> {
    let until = Instant::now() + duration;
    loop {
        check(stop)?;
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(());
        }
        thread::sleep(left.min(Duration::from_millis(25)));
    }
}
pub fn is_cancelled(error: &anyhow::Error) -> bool {
    error.is::<Cancelled>()
}

/// Cancel pending synchronous driver I/O on this worker while a native call runs.
/// The helper is joined before returning, so it cannot affect later I/O.
pub fn synchronous_io<T>(stop: &AtomicBool, call: impl FnOnce() -> T) -> Result<T> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::{
        IO::CancelSynchronousIo,
        Threading::{GetCurrentThreadId, OpenThread, THREAD_TERMINATE},
    };
    check(stop)?;
    let raw = unsafe { OpenThread(THREAD_TERMINATE, 0, GetCurrentThreadId()) };
    if raw.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let finished = AtomicBool::new(false);
    struct Complete<'a>(&'a AtomicBool);
    impl Drop for Complete<'_> {
        fn drop(&mut self) {
            self.0.store(true, Ordering::Release);
        }
    }
    let result = thread::scope(|scope| {
        scope.spawn(|| {
            while !finished.load(Ordering::Acquire) {
                if stop.load(Ordering::Acquire) {
                    // ERROR_NOT_FOUND is expected if the request has not begun
                    // or completed between the flag check and this call.
                    unsafe { CancelSynchronousIo(handle.as_raw_handle()) };
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let complete = Complete(&finished);
        let result = call();
        drop(complete);
        result
    });
    check(stop)?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_is_not_a_device_error() {
        let flag = AtomicBool::new(true);
        assert!(is_cancelled(&check(&flag).unwrap_err()));
        assert!(!is_cancelled(&anyhow::anyhow!("device missing")));
        let before = Instant::now();
        assert!(pause(&flag, Duration::from_secs(2)).is_err());
        assert!(before.elapsed() < Duration::from_millis(50));
    }
}
