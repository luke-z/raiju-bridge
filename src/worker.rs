use crate::{
    accuracy::{self, StickMonitor, StickSnapshot, SweepResult},
    cancel,
    measure::{self, ButtonMonitor, ButtonSample, Stats},
    native::{self, ExclusiveSession, VirtualPad},
    protocol,
};
use anyhow::{Result, bail};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Bridge,
    Buttons,
    Benchmark,
    Sticks,
    Sweep,
}
impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Bridge => "bridge",
            Self::Buttons => "buttons",
            Self::Benchmark => "benchmark",
            Self::Sticks => "sticks",
            Self::Sweep => "stick-sweep",
        }
    }
}
#[derive(Debug)]
pub enum Message {
    Status(String),
    Connected(String),
    Reports(u64, f64, f64, bool),
    Button(ButtonSample),
    Sticks(StickSnapshot),
    Diagnostics(Box<crate::diagnostics::Snapshot>),
}
#[derive(Debug, Default)]
pub struct SessionReport {
    pub statistics: Option<Stats>,
    pub sticks: Option<StickSnapshot>,
    pub sweep: Option<SweepResult>,
    pub directory: Option<PathBuf>,
    pub diagnostics: Option<Box<crate::diagnostics::Snapshot>>,
}
#[derive(Debug)]
pub enum Outcome {
    Stopped,
    Completed(Box<SessionReport>),
    Failed(String),
}
fn outcome(result: Result<SessionReport>) -> Outcome {
    match result {
        Ok(r) => Outcome::Completed(Box::new(r)),
        Err(e) if cancel::is_cancelled(&e) => Outcome::Stopped,
        Err(e) => Outcome::Failed(format!("{e:#}")),
    }
}
fn send(tx: &SyncSender<Message>, message: Message) {
    let _ = tx.try_send(message);
}
pub struct Worker {
    stop: Arc<AtomicBool>,
    diagnostics: Arc<AtomicBool>,
    thread: Option<JoinHandle<Outcome>>,
    pub receiver: Receiver<Message>,
}
impl Worker {
    pub fn start(mode: Mode, directory: PathBuf) -> Self {
        let (tx, receiver) = mpsc::sync_channel(256);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let diagnostics = Arc::new(AtomicBool::new(false));
        let diagnostic_flag = diagnostics.clone();
        let thread = thread::spawn(move || {
            outcome((|| -> Result<SessionReport> {
                cancel::check(&flag)?;
                send(&tx, Message::Status("Connecting…".into()));
                match mode {
                    Mode::Benchmark => {
                        let statistics = measure::benchmark(&directory, 1000, 3, 1, flag, |s| {
                            send(&tx, Message::Status(s))
                        })?;
                        Ok(SessionReport {
                            statistics: Some(statistics),
                            directory: Some(directory),
                            ..Default::default()
                        })
                    }
                    Mode::Sweep => {
                        let sweep =
                            accuracy::sweep(&directory, &flag, |s| send(&tx, Message::Status(s)))?;
                        Ok(SessionReport {
                            sweep: Some(sweep),
                            directory: Some(directory),
                            ..Default::default()
                        })
                    }
                    _ => {
                        let _exclusive = ExclusiveSession::acquire()?;
                        loop {
                            cancel::check(&flag)?;
                            match run_bridge(mode, &directory, flag.clone(), &diagnostic_flag, &tx)
                            {
                                Err(e)
                                    if mode == Mode::Bridge
                                        && (e.is::<native::SourceUnavailable>()
                                            || e.is::<Disconnected>()) =>
                                {
                                    send(
                                        &tx,
                                        Message::Status(
                                            "Waiting for Raiju · connect it in PC or PS5 mode"
                                                .into(),
                                        ),
                                    );
                                    cancel::pause(&flag, Duration::from_secs(1))?;
                                }
                                other => return other,
                            }
                        }
                    }
                }
            })())
        });
        Self {
            stop,
            diagnostics,
            thread: Some(thread),
            receiver,
        }
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn diagnostics(&self, enabled: bool) {
        self.diagnostics.store(enabled, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
    // Completion travels in the join result, never through the bounded status queue.
    // A busy/minimized UI therefore cannot lose stop, success or error notifications.
    pub fn take_outcome(&mut self) -> Option<Outcome> {
        if !self.is_finished() {
            return None;
        }
        self.thread.take().map(|h| {
            h.join().unwrap_or_else(|_| {
                Outcome::Failed("The bridge worker stopped unexpectedly.".into())
            })
        })
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
#[derive(Debug)]
struct Disconnected;
impl std::fmt::Display for Disconnected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Raiju disconnected or stopped sending input")
    }
}
impl std::error::Error for Disconnected {}

fn run_bridge(
    mode: Mode,
    directory: &Path,
    stop: Arc<AtomicBool>,
    diagnostics: &AtomicBool,
    tx: &SyncSender<Message>,
) -> Result<SessionReport> {
    let test = mode != Mode::Bridge;
    cancel::check(&stop)?;
    let (mut input, name) = crate::input::Source::open()?;
    let pc_mode = input.is_pc();
    cancel::check(&stop)?;
    let mut buffer = [0u8; 128];
    let first_deadline = Instant::now() + Duration::from_secs(2);
    let first = loop {
        cancel::check(&stop)?;
        if let Some(frame) = input.read(&mut buffer)? {
            break frame.state;
        }
        if Instant::now() >= first_deadline {
            return Err(Disconnected.into());
        }
    };
    if test {
        std::fs::create_dir_all(directory)?;
        std::fs::write(
            directory.join("input-source.json"),
            serde_json::to_vec_pretty(
                &serde_json::json!({"source":name,"pc_mode":pc_mode,"pc_poll_target_hz":if pc_mode{Some(2000)}else{None},"note":"PC polling frequency is an application sampling rate, not proof of USB report frequency. PC axes are rounded to the nearest 8-bit DualSense value with center and endpoints preserved."}),
            )?,
        )?;
    }
    send(tx, Message::Connected(name));
    let previous = native::sony_paths()?;
    cancel::check(&stop)?;
    let pad = VirtualPad::attach(1, &stop)?;
    cancel::check(&stop)?;
    let virtual_input = native::wait_for_virtual(&previous, &stop)?;
    let virtual_path = virtual_input.get_device_info()?.path().to_owned();
    input.begin_forwarding();
    cancel::check(&stop)?;
    let origin = Instant::now();
    let buttons = Arc::new(Mutex::new(ButtonMonitor::new(origin)));
    let sticks = if mode == Mode::Sticks {
        Some(Arc::new(Mutex::new(StickMonitor::new(directory)?)))
    } else {
        None
    };
    let observer_stop = Arc::new(AtomicBool::new(false));
    let observer_error = Arc::new(Mutex::new(None::<String>));
    let observer = if test {
        let buttons = buttons.clone();
        let sticks = sticks.clone();
        let flag = observer_stop.clone();
        let error = observer_error.clone();
        let tx = tx.clone();
        Some(thread::spawn(move || {
            let mut report = [0u8; 128];
            let mut last = None;
            while !flag.load(Ordering::Acquire) {
                let count = match virtual_input.read_timeout(&mut report, 50) {
                    Ok(n) => n,
                    Err(e) => {
                        *error.lock().unwrap() = Some(e.to_string());
                        break;
                    }
                };
                let arrived = Instant::now();
                if count == 0 {
                    continue;
                }
                let Some(state) = protocol::virtual_signature(&report[..count]) else {
                    *error.lock().unwrap() = Some("Unsupported virtual input report".into());
                    break;
                };
                if let Some(sticks) = &sticks {
                    sticks
                        .lock()
                        .unwrap()
                        .observe(accuracy::axes(&report[..count]).unwrap(), arrived);
                } else if let Some(old) = last
                    && old != state
                    && let Some(sample) = buttons.lock().unwrap().observe(old, state, arrived)
                {
                    send(&tx, Message::Button(sample));
                }
                last = Some(state);
            }
        }))
    } else {
        drop(virtual_input);
        None
    };
    send(
        tx,
        Message::Status(
            if test {
                "Preparing test…"
            } else {
                "Connected · DualSense ready"
            }
            .into(),
        ),
    );
    let mut live: Option<crate::diagnostics::Live> = None;
    let mut retiring: Vec<crate::diagnostics::Live> = Vec::new();
    let mut live_result = None;
    let result = (|| -> Result<()> {
        let mut last_signature = protocol::signature(&first);
        let mut ready = !test;
        let mut last_input = Instant::now();
        let mut report_time = last_input;
        let mut stick_time = last_input;
        let mut reports = 0u64;
        let mut interval_reports = 0u64;
        let mut interval_fresh = 0u64;
        let mut first_forward = true;
        loop {
            if stop.load(Ordering::Acquire) {
                break;
            }
            if mode == Mode::Bridge {
                let enabled = diagnostics.load(Ordering::Acquire);
                if enabled && live.is_none() {
                    live = Some(crate::diagnostics::Live::start(
                        virtual_path.clone(),
                        tx.clone(),
                    ));
                } else if !enabled && let Some(reader) = live.take() {
                    reader.stop();
                    retiring.push(reader);
                }
                let mut index = 0;
                while index < retiring.len() {
                    if retiring[index].is_finished() {
                        live_result = Some(Box::new(retiring.swap_remove(index).finish()));
                    } else {
                        index += 1;
                    }
                }
            }
            if let Some(error) = observer_error.lock().unwrap().as_ref() {
                bail!("Virtual input reader failed: {error}");
            }
            let frame = match input.read(&mut buffer) {
                Ok(frame) => frame,
                Err(e) => {
                    cancel::check(&stop)?;
                    if pc_mode && !e.is::<native::SourceUnavailable>() {
                        return Err(e);
                    }
                    return Err(Disconnected.into());
                }
            };
            let Some(frame) = frame else {
                if Instant::now().duration_since(last_input) > Duration::from_millis(750) {
                    return Err(Disconnected.into());
                }
                continue;
            };
            let received = frame.received;
            last_input = received;
            let state = frame.state;
            let signature = protocol::signature(&state);
            if let Some(live) = &live {
                live.capture(&frame);
            }
            if test {
                if !ready && received.duration_since(origin) >= Duration::from_secs(1) {
                    ready = true;
                    buttons.lock().unwrap().ready = true;
                    if let Some(sticks) = &sticks {
                        sticks.lock().unwrap().ready = true;
                    }
                    send(
                        tx,
                        Message::Status(
                            if mode == Mode::Sticks {
                                "Move both sticks slowly through their full range, then Stop"
                            } else {
                                "Tap buttons, then Stop to save"
                            }
                            .into(),
                        ),
                    );
                }
                if let Some(sticks) = &sticks {
                    sticks.lock().unwrap().source(frame.axes(), received);
                } else if ready && signature != last_signature {
                    buttons
                        .lock()
                        .unwrap()
                        .source(last_signature, signature, received);
                }
                last_signature = signature;
            }
            if frame.fresh || first_forward {
                pad.set(state)?;
                first_forward = false;
            }
            reports += 1;
            interval_reports += 1;
            interval_fresh += u64::from(frame.fresh);
            let elapsed = received.duration_since(report_time).as_secs_f64();
            if elapsed >= 1.0 {
                send(
                    tx,
                    Message::Reports(
                        reports,
                        interval_reports as f64 / elapsed,
                        interval_fresh as f64 / elapsed,
                        pc_mode,
                    ),
                );
                interval_reports = 0;
                interval_fresh = 0;
                report_time = received;
            }
            if let Some(sticks) = &sticks
                && received.duration_since(stick_time) >= Duration::from_millis(100)
            {
                send(tx, Message::Sticks(sticks.lock().unwrap().snapshot.clone()));
                stick_time = received;
            }
        }
        Ok(())
    })();
    for reader in retiring {
        live_result = Some(Box::new(reader.finish()));
    }
    if let Some(reader) = live {
        live_result = Some(Box::new(reader.finish()));
    }
    observer_stop.store(true, Ordering::Release);
    if let Some(observer) = observer
        && observer.join().is_err()
    {
        bail!("Diagnostic observer stopped unexpectedly");
    }
    let mut report = SessionReport {
        diagnostics: live_result,
        ..Default::default()
    };
    if mode == Mode::Buttons {
        let mut monitor = buttons.lock().unwrap();
        monitor.reader_error = observer_error.lock().unwrap().clone();
        if let Err(e) = &result {
            monitor.reader_error = Some(format!("{e:#}"));
        }
        report.statistics = Some(monitor.save(directory)?);
        report.directory = Some(directory.into());
    }
    if let Some(sticks) = sticks {
        let mut monitor = sticks.lock().unwrap();
        if let Err(e) = &result {
            monitor.reader_error = Some(format!("{e:#}"));
        }
        monitor.save()?;
        report.sticks = Some(monitor.snapshot.clone());
        report.directory = Some(directory.into());
    }
    result?;
    Ok(report)
}
pub fn results_root() -> PathBuf {
    crate::desktop::data_root().join("results")
}
pub fn session_directory(mode: Mode) -> PathBuf {
    results_root().join(format!(
        "{}-{}",
        mode.name(),
        chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_result_survives_a_full_status_queue() {
        let (tx, receiver) = mpsc::sync_channel(1);
        tx.send(Message::Status("busy".into())).unwrap();
        send(&tx, Message::Status("dropped progress".into()));
        let thread = thread::spawn(|| Outcome::Stopped);
        let mut worker = Worker {
            stop: Arc::new(AtomicBool::new(false)),
            diagnostics: Arc::new(AtomicBool::new(false)),
            thread: Some(thread),
            receiver,
        };
        while !worker.is_finished() {
            thread::yield_now();
        }
        assert!(matches!(worker.take_outcome(), Some(Outcome::Stopped)));
    }
    #[test]
    fn explicit_stop_and_genuine_errors_remain_distinct() {
        assert!(matches!(
            outcome(Err(cancel::Cancelled.into())),
            Outcome::Stopped
        ));
        assert!(matches!(
            outcome(Err(anyhow::anyhow!("USB failed"))),
            Outcome::Failed(_)
        ));
    }
}
