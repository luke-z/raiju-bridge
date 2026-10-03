use crate::{
    native::{self, ExclusiveSession, VirtualPad},
    protocol::{self, PadState},
};
use anyhow::{Result, bail};
use serde::Serialize;
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Stats {
    pub count: usize,
    pub median_ms: Option<f64>,
    pub mean_ms: Option<f64>,
    pub p95_ms: Option<f64>,
    pub p99_ms: Option<f64>,
    pub max_ms: Option<f64>,
}
pub fn stats(values: impl IntoIterator<Item = f64>) -> Stats {
    let mut values: Vec<_> = values.into_iter().collect();
    values.sort_by(f64::total_cmp);
    if values.is_empty() {
        return Stats::default();
    }
    let p = |x: f64| values[(x * values.len() as f64).ceil() as usize - 1];
    Stats {
        count: values.len(),
        median_ms: Some(p(0.5)),
        mean_ms: Some(values.iter().sum::<f64>() / values.len() as f64),
        p95_ms: Some(p(0.95)),
        p99_ms: Some(p(0.99)),
        max_ms: values.last().copied(),
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct ButtonSample {
    pub index: usize,
    pub action: String,
    pub press: bool,
    pub release: bool,
    pub delay_ms: f64,
}
struct Edge {
    index: usize,
    time: Instant,
    state: u32,
    pressed: u32,
    released: u32,
}
#[derive(Serialize)]
struct Unexpected {
    elapsed_ms: f64,
    from: u32,
    to: u32,
    pending: Vec<u32>,
}

pub struct ButtonMonitor {
    pub ready: bool,
    origin: Instant,
    pending: VecDeque<Edge>,
    pub samples: Vec<ButtonSample>,
    source_edges: usize,
    pub missed: usize,
    unexpected: Vec<Unexpected>,
    pub reader_error: Option<String>,
}
impl ButtonMonitor {
    pub fn new(origin: Instant) -> Self {
        Self {
            ready: false,
            origin,
            pending: VecDeque::with_capacity(64),
            samples: Vec::with_capacity(4096),
            source_edges: 0,
            missed: 0,
            unexpected: vec![],
            reader_error: None,
        }
    }
    pub fn source(&mut self, old: u32, state: u32, time: Instant) {
        if !self.ready {
            return;
        }
        while self
            .pending
            .front()
            .is_some_and(|e| time.duration_since(e.time).as_secs_f64() > 2.0)
        {
            self.pending.pop_front();
            self.missed += 1;
        }
        self.source_edges += 1;
        self.pending.push_back(Edge {
            index: self.source_edges,
            time,
            state,
            pressed: state & !old,
            released: old & !state,
        });
    }
    pub fn observe(&mut self, old: u32, state: u32, time: Instant) -> Option<ButtonSample> {
        if !self.ready {
            return None;
        }
        let Some(index) = self
            .pending
            .iter()
            .position(|e| e.state == state && e.time <= time)
        else {
            self.unexpected.push(Unexpected {
                elapsed_ms: time.duration_since(self.origin).as_secs_f64() * 1000.0,
                from: old,
                to: state,
                pending: self.pending.iter().map(|e| e.state).collect(),
            });
            return None;
        };
        let last = self
            .pending
            .iter()
            .rposition(|e| e.state == state && e.time <= time)
            .unwrap();
        if last != index {
            self.pending.drain(..=last);
            self.missed += last + 1;
            return None;
        }
        self.pending.drain(..index);
        self.missed += index;
        let edge = self.pending.pop_front().unwrap();
        let delay_ms = time.duration_since(edge.time).as_secs_f64() * 1000.0;
        if delay_ms > 2000.0 {
            self.missed += 1;
            return None;
        }
        let mut parts = Vec::with_capacity(2);
        if edge.pressed != 0 {
            parts.push(format!("press {}", protocol::button_names(edge.pressed)));
        }
        if edge.released != 0 {
            parts.push(format!("release {}", protocol::button_names(edge.released)));
        }
        let sample = ButtonSample {
            index: edge.index,
            action: parts.join("; "),
            press: edge.pressed != 0,
            release: edge.released != 0,
            delay_ms,
        };
        self.samples.push(sample.clone());
        Some(sample)
    }
    pub fn save(&self, directory: &Path) -> Result<Stats> {
        fs::create_dir_all(directory)?;
        let presses = stats(self.samples.iter().filter(|s| s.press).map(|s| s.delay_ms));
        let releases = stats(
            self.samples
                .iter()
                .filter(|s| s.release)
                .map(|s| s.delay_ms),
        );
        let all = stats(self.samples.iter().map(|s| s.delay_ms));
        let report = serde_json::json!({
            "recorded_at": chrono::Local::now(), "implementation": "Rust / GPUI / VIIPER 0.8.2 / USBip 0.9.8.1",
            "method": "Physical source read completion (HID in PS5 mode, XInput in PC mode), before translation and submission, to matching virtual DualSense HID read completion. UI not in forwarding path; see input-source.json.",
            "scope": "Added software delay only. Excludes physical switch, firmware scanning, physical USB delivery before the source read, game and display latency.",
            "source_edges": self.source_edges, "matched_edges": self.samples.len(), "not_observed_or_ambiguous": self.missed,
            "pending_at_stop": self.pending.len(), "unexpected_virtual_edges": self.unexpected.len(),
            "unexpected_details": self.unexpected, "reader_error": self.reader_error,
            "presses": presses, "releases": releases, "all": all
        });
        fs::write(
            directory.join("button-results.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        let mut csv = File::create(directory.join("button-samples.csv"))?;
        writeln!(csv, "index,action,has_press,has_release,delay_ms")?;
        for s in &self.samples {
            writeln!(
                csv,
                "{},{},{},{},{:.6}",
                s.index, s.action, s.press, s.release, s.delay_ms
            )?;
        }
        Ok(all)
    }
}

fn token_state(token: u32) -> PadState {
    PadState {
        lx: ((token & 15) as i8) - 8,
        ly: (((token >> 4) & 15) as i8) - 8,
        rx: (((token >> 8) & 15) as i8) - 8,
        ry: (((token >> 12) & 15) as i8) - 8,
        ..Default::default()
    }
}
fn report_token(report: &[u8]) -> Option<u32> {
    if report.len() != 64 || report[0] != 1 || report[1..5].iter().any(|b| !(120..136).contains(b))
    {
        return None;
    }
    Some(
        (report[1] - 120) as u32
            | (((report[2] - 120) as u32) << 4)
            | (((report[3] - 120) as u32) << 8)
            | (((report[4] - 120) as u32) << 12),
    )
}

pub fn benchmark(
    directory: &Path,
    samples_per_run: usize,
    runs: usize,
    flush: u32,
    stop: Arc<AtomicBool>,
    log: impl Fn(String),
) -> Result<Stats> {
    crate::cancel::check(&stop)?;
    if samples_per_run == 0
        || runs == 0
        || samples_per_run
            .checked_add(100)
            .and_then(|n| n.checked_mul(runs))
            .is_none_or(|n| n >= 32768)
    {
        bail!("Invalid benchmark sample count.");
    }
    let _exclusive = ExclusiveSession::acquire()?;
    fs::create_dir_all(directory)?;
    let started_at = chrono::Local::now();
    let previous = native::sony_paths()?;
    crate::cancel::check(&stop)?;
    let pad = VirtualPad::attach(flush, &stop)?;
    let input = native::wait_for_virtual(&previous, &stop)?;
    let expected = Arc::new(AtomicU32::new(0));
    let reader_stop = Arc::new(AtomicBool::new(false));
    let (tx, rx) = mpsc::sync_channel::<Result<(u32, Instant), String>>(32);
    let reader = {
        let expected = expected.clone();
        let reader_stop = reader_stop.clone();
        thread::spawn(move || {
            let mut buffer = [0u8; 128];
            let mut delivered = 0;
            while !reader_stop.load(Ordering::Relaxed) {
                let count = match input.read_timeout(&mut buffer, 100) {
                    Ok(n) => n,
                    Err(e) => {
                        let _ = tx.send(Err(e.to_string()));
                        break;
                    }
                };
                let arrival = Instant::now();
                if count == 0 {
                    continue;
                }
                if let Some(token) = report_token(&buffer[..count])
                    && token == expected.load(Ordering::Acquire)
                    && token != delivered
                {
                    delivered = token;
                    if tx.send(Ok((token, arrival))).is_err() {
                        break;
                    }
                }
            }
        })
    };
    #[derive(Serialize)]
    struct Sample {
        run: usize,
        index: usize,
        token: u32,
        delivery_ms: f64,
        submit_ms: f64,
    }
    let result = (|| -> Result<Stats> {
        let mut samples = Vec::with_capacity(samples_per_run * runs);
        let mut token = 0u32;
        let mut random = 20261003u32;
        log(format!(
            "Virtual DualSense ready. {runs} runs × {samples_per_run} measured changes; 100 warmups per run."
        ));
        for run in 1..=runs {
            for index in 0..(samples_per_run + 100) {
                crate::cancel::check(&stop)?;
                random = random.wrapping_mul(1664525).wrapping_add(1013904223);
                thread::sleep(Duration::from_millis(1 + (random % 5) as u64));
                token += 1;
                let state = token_state(token);
                expected.store(token, Ordering::Release);
                let sent = Instant::now();
                pad.set(state)?;
                let returned = Instant::now();
                let deadline = Instant::now() + Duration::from_secs(2);
                let (actual, received) = loop {
                    crate::cancel::check(&stop)?;
                    match rx.recv_timeout(Duration::from_millis(50)) {
                        Ok(value) => break value.map_err(anyhow::Error::msg)?,
                        Err(mpsc::RecvTimeoutError::Timeout) if Instant::now() < deadline => {}
                        Err(e) => return Err(e.into()),
                    }
                };
                if actual != token || received < sent {
                    bail!("Invalid sequence correlation.");
                }
                if index >= 100 {
                    samples.push(Sample {
                        run,
                        index: index - 99,
                        token,
                        delivery_ms: received.duration_since(sent).as_secs_f64() * 1000.0,
                        submit_ms: returned.duration_since(sent).as_secs_f64() * 1000.0,
                    });
                }
                if index >= 100 && (index - 99) % 500 == 0 {
                    log(format!(
                        "Run {run}: {}/{} received",
                        index - 99,
                        samples_per_run
                    ));
                }
            }
        }
        let combined = stats(samples.iter().map(|s| s.delivery_ms));
        let submit = stats(samples.iter().map(|s| s.submit_ms));
        let report = serde_json::json!({ "started_at": started_at, "finished_at": chrono::Local::now(),
            "backend": "VIIPER 0.8.2 / USBip 0.9.8.1 / virtual DualSense", "implementation": "Rust",
            "transport": "localhost only", "flush_interval_ms": flush, "warmups_per_run": 100,
            "method": "Instant immediately before native state submission to Instant immediately after matching Windows HID read. Unique token in small stick offsets; one outstanding change.",
            "scope": "Added software path, not controller hardware or game/display latency. Normal thread priority; no timer-resolution override; seeded requested 1-5 ms sleeps outside measurement.",
            "failed_or_timed_out_samples": 0, "combined": combined, "native_submission": submit });
        fs::write(
            directory.join("latency-results.json"),
            serde_json::to_vec_pretty(&report)?,
        )?;
        let mut csv = File::create(directory.join("latency-samples.csv"))?;
        writeln!(csv, "run,index,token,delivery_ms,submit_ms")?;
        for s in samples {
            writeln!(
                csv,
                "{},{},{},{:.6},{:.6}",
                s.run, s.index, s.token, s.delivery_ms, s.submit_ms
            )?;
        }
        log(format!(
            "Median {:.3} ms · p99 {:.3} ms · max {:.3} ms",
            combined.median_ms.unwrap(),
            combined.p99_ms.unwrap(),
            combined.max_ms.unwrap()
        ));
        Ok(combined)
    })();
    reader_stop.store(true, Ordering::Relaxed);
    drop(rx);
    let _ = reader.join();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn paired_and_unexpected_are_separate() {
        let origin = Instant::now();
        let mut m = ButtonMonitor::new(origin);
        m.ready = true;
        m.source(0, 0x20, origin);
        assert!(
            m.observe(0, 0x20, origin + Duration::from_micros(80))
                .is_some()
        );
        assert_eq!(m.samples[0].delay_ms, 0.08);
        assert!(
            m.observe(0x20, 0, origin + Duration::from_micros(90))
                .is_none()
        );
        assert_eq!(m.unexpected.len(), 1);
    }
    #[test]
    fn ambiguous_repeated_states_are_not_mistimed() {
        let t = Instant::now();
        let mut m = ButtonMonitor::new(t);
        m.ready = true;
        m.source(0, 0x20, t);
        m.source(0x20, 0, t);
        m.source(0, 0x20, t);
        assert!(m.observe(0, 0x20, t + Duration::from_millis(1)).is_none());
        assert_eq!(m.missed, 3);
        assert!(m.samples.is_empty());
    }
}
