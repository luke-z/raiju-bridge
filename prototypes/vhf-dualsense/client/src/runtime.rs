//! Headless, cancellable PS-mode forwarding. No GPUI, shell or helper process.
use anyhow::Result;
use hidapi::HidApi;
use std::{
    io::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

const ENABLED: u8 = 1;
const SUSPENDED: u8 = 2;
const QUIT: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Off,
    Waiting,
    PcMode,
    Multiple,
    Running,
    Error,
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Waiting => "Waiting for a Raiju in PS mode",
            Self::PcMode => "PC mode: native input; switch to PS mode for PlayStation prompts",
            Self::Multiple => "Connect only one Raiju in PS mode",
            Self::Running => "Running: PlayStation controller input",
            Self::Error => "Connection problem: retrying",
        }
    }
}

struct Shared {
    flags: AtomicU8,
    state: Mutex<State>,
}
impl Shared {
    fn cancelled(&self) -> bool {
        self.flags.load(Ordering::Acquire) != ENABLED
    }
    fn publish(&self, next: State, notify: &impl Fn()) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if *state != next {
            *state = next;
            drop(state);
            notify();
        }
    }
}

pub struct BridgeRuntime {
    shared: Arc<Shared>,
    wake: SyncSender<()>,
    worker: Option<JoinHandle<()>>,
}
impl BridgeRuntime {
    pub fn start(enabled: bool, notify: impl Fn() + Send + 'static) -> Result<Self> {
        Self::with_backend(enabled, Hardware, notify, Duration::from_millis(750))
    }
    fn with_backend(
        enabled: bool,
        mut backend: impl Backend + Send + 'static,
        notify: impl Fn() + Send + 'static,
        interval: Duration,
    ) -> Result<Self> {
        let shared = Arc::new(Shared {
            flags: AtomicU8::new(if enabled { ENABLED } else { 0 }),
            state: Mutex::new(if enabled { State::Waiting } else { State::Off }),
        });
        let (wake, receive) = mpsc::sync_channel(1);
        let state = shared.clone();
        let worker = thread::Builder::new()
            .name("raiju-input".into())
            .spawn(move || {
                worker(&state, &receive, &mut backend, notify, interval);
            })?;
        Ok(Self {
            shared,
            wake,
            worker: Some(worker),
        })
    }
    pub fn state(&self) -> State {
        *self.shared.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn enabled(&self) -> bool {
        self.shared.flags.load(Ordering::Acquire) & ENABLED != 0
    }
    pub fn set_enabled(&self, enabled: bool) {
        if enabled {
            self.shared.flags.fetch_or(ENABLED, Ordering::AcqRel);
        } else {
            self.shared.flags.fetch_and(!ENABLED, Ordering::AcqRel);
        }
        self.wake();
    }
    pub fn suspend(&self, suspended: bool) {
        if suspended {
            self.shared.flags.fetch_or(SUSPENDED, Ordering::AcqRel);
        } else {
            self.shared.flags.fetch_and(!SUSPENDED, Ordering::AcqRel);
        }
        self.wake();
    }
    pub fn wake(&self) {
        let _ = self.wake.try_send(());
    }
    pub fn stop(&self) {
        self.shared.flags.fetch_or(QUIT, Ordering::AcqRel);
        self.wake();
    }
}
impl Drop for BridgeRuntime {
    fn drop(&mut self) {
        self.stop();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    None,
    Ps,
    Pc,
    Multiple,
}
trait Backend {
    fn source(&mut self) -> Result<Source>;
    fn run(&mut self, cancel: &dyn Fn() -> bool, running: &dyn Fn()) -> Result<()>;
}
struct Hardware;
impl Backend for Hardware {
    fn source(&mut self) -> Result<Source> {
        let api = HidApi::new()?;
        let mut ps = 0;
        let mut pc = false;
        for d in api.device_list().filter(|d| d.vendor_id() == 0x1532) {
            if matches!(d.product_id(), 0x1024 | 0x1026)
                && d.usage_page() == 1
                && matches!(d.usage(), 4 | 5)
            {
                ps += 1;
            }
            pc |= matches!(d.product_id(), 0x1025 | 0x1027);
        }
        Ok(match (ps, pc) {
            (0, false) => Source::None,
            (0, true) => Source::Pc,
            (1, _) => Source::Ps,
            _ => Source::Multiple,
        })
    }
    fn run(&mut self, cancel: &dyn Fn() -> bool, running: &dyn Fn()) -> Result<()> {
        super::forward("dualsense", cancel, running, |_| {})
    }
}

fn worker(
    state: &Shared,
    wake: &Receiver<()>,
    backend: &mut impl Backend,
    notify: impl Fn(),
    interval: Duration,
) {
    let mut last_error = String::new();
    while state.flags.load(Ordering::Acquire) & QUIT == 0 {
        if state.cancelled() {
            state.publish(State::Off, &notify);
        } else {
            let attempt: Result<()> = (|| {
                match backend.source()? {
                    Source::None => state.publish(State::Waiting, &notify),
                    Source::Pc => state.publish(State::PcMode, &notify),
                    Source::Multiple => state.publish(State::Multiple, &notify),
                    Source::Ps => {
                        backend.run(&|| state.cancelled(), &|| {
                            state.publish(State::Running, &notify)
                        })?;
                        state.publish(State::Waiting, &notify);
                    }
                }
                Ok(())
            })();
            if !state.cancelled() {
                match attempt {
                    Ok(()) => {
                        last_error.clear();
                    }
                    Err(error) => {
                        let message = format!("{error:#}");
                        if last_error != message {
                            log(&message);
                            last_error = message;
                        }
                        state.publish(State::Error, &notify);
                    }
                }
            }
        }
        // After disabling/suspending a live session, publish Off immediately,
        // after the backend has removed its virtual child, rather than waiting.
        if state.cancelled() {
            state.publish(State::Off, &notify);
        }
        let _ = wake.recv_timeout(interval);
    }
}

pub fn log(message: &str) {
    let Some(root) = std::env::var_os("LOCALAPPDATA") else {
        return;
    };
    let folder = std::path::PathBuf::from(root).join("RaijuBridge");
    if std::fs::create_dir_all(&folder).is_err() {
        return;
    }
    let path = folder.join("bridge.log");
    let full = path.metadata().is_ok_and(|m| m.len() > 65536);
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!full)
        .truncate(full)
        .open(path)
    {
        let seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let _ = writeln!(file, "{seconds}: {message}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize};

    struct Fake {
        source: Arc<Mutex<Source>>,
        active: Arc<AtomicBool>,
        sessions: Arc<AtomicUsize>,
    }
    impl Backend for Fake {
        fn source(&mut self) -> Result<Source> {
            Ok(*self.source.lock().unwrap())
        }
        fn run(&mut self, cancel: &dyn Fn() -> bool, running: &dyn Fn()) -> Result<()> {
            self.active.store(true, Ordering::Release);
            self.sessions.fetch_add(1, Ordering::AcqRel);
            running();
            while !cancel() && self.source()? == Source::Ps {
                thread::sleep(Duration::from_millis(2));
            }
            self.active.store(false, Ordering::Release);
            Ok(())
        }
    }
    fn wait(check: impl Fn() -> bool) {
        let start = std::time::Instant::now();
        while !check() {
            assert!(
                start.elapsed() < Duration::from_secs(3),
                "Worker transition timed out"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[test]
    fn toggle_disconnect_pc_mode_suspend_and_shutdown_release_the_session() {
        let source = Arc::new(Mutex::new(Source::Ps));
        let active = Arc::new(AtomicBool::new(false));
        let sessions = Arc::new(AtomicUsize::new(0));
        let runtime = BridgeRuntime::with_backend(
            false,
            Fake {
                source: source.clone(),
                active: active.clone(),
                sessions: sessions.clone(),
            },
            || {},
            Duration::from_millis(5),
        )
        .unwrap();
        assert_eq!(runtime.state(), State::Off);
        assert_eq!(sessions.load(Ordering::Acquire), 0);
        runtime.set_enabled(true);
        wait(|| runtime.state() == State::Running);
        runtime.set_enabled(false);
        wait(|| runtime.state() == State::Off);
        assert!(!active.load(Ordering::Acquire));
        *source.lock().unwrap() = Source::Pc;
        runtime.set_enabled(true);
        wait(|| runtime.state() == State::PcMode);
        assert!(!active.load(Ordering::Acquire));
        assert_eq!(sessions.load(Ordering::Acquire), 1);
        *source.lock().unwrap() = Source::None;
        wait(|| runtime.state() == State::Waiting);
        *source.lock().unwrap() = Source::Multiple;
        wait(|| runtime.state() == State::Multiple);
        *source.lock().unwrap() = Source::Ps;
        wait(|| runtime.state() == State::Running);
        runtime.suspend(true);
        wait(|| runtime.state() == State::Off);
        assert!(!active.load(Ordering::Acquire));
        runtime.suspend(false);
        wait(|| runtime.state() == State::Running);
        *source.lock().unwrap() = Source::None;
        wait(|| runtime.state() == State::Waiting);
        assert!(!active.load(Ordering::Acquire));
        *source.lock().unwrap() = Source::Ps;
        wait(|| runtime.state() == State::Running);
        drop(runtime);
        assert!(!active.load(Ordering::Acquire));
    }
}
