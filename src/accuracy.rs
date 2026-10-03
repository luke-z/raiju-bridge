use crate::{
    cancel,
    native::{self, ExclusiveSession, VirtualPad},
    protocol,
};
use anyhow::{Context, Result, bail};
use serde::Serialize;
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

pub fn axes(report: &[u8]) -> Option<[u8; 4]> {
    (report.len() >= 5 && report[0] == 1).then(|| [report[1], report[2], report[3], report[4]])
}
pub fn sweep_cases() -> Vec<[u8; 4]> {
    let mut cases = Vec::with_capacity(1800);
    for axis in 0..4 {
        for value in 0..=255 {
            let mut row = [128; 4];
            row[axis] = value;
            cases.push(row);
        }
    }
    for mask in 0..16 {
        cases.push(std::array::from_fn(|axis| {
            if mask & (1 << axis) == 0 { 0 } else { 255 }
        }));
    }
    for value in 0..=255 {
        cases.push([value, 255 - value, 255 - value, value]);
    }
    let mut rng = 20261003u32;
    for _ in 0..512 {
        rng = rng.wrapping_mul(1664525).wrapping_add(1013904223);
        cases.push(rng.to_le_bytes());
    }
    cases.push([128; 4]);
    cases
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct SweepResult {
    pub tested: usize,
    pub mismatches: usize,
    pub max_error: u8,
}
pub fn sweep(directory: &Path, stop: &AtomicBool, log: impl Fn(String)) -> Result<SweepResult> {
    cancel::check(stop)?;
    let _exclusive = ExclusiveSession::acquire()?;
    let previous = native::sony_paths()?;
    cancel::check(stop)?;
    let pad = VirtualPad::attach(1, stop)?;
    let input = native::wait_for_virtual(&previous, stop)?;
    fs::create_dir_all(directory)?;
    let mut csv = BufWriter::new(File::create(directory.join("stick-sweep.csv"))?);
    writeln!(
        csv,
        "sequence,source_mode,lx_in,ly_in,rx_in,ry_in,lx_out,ly_out,rx_out,ry_out,max_error"
    )?;
    let mut summary = SweepResult::default();
    let mut report = [0u8; 128];
    let result = (|| -> Result<()> {
        let cases = sweep_cases();
        log(format!(
            "Checking {} stick vectors through Windows…",
            cases.len() * 2
        ));
        for (index, expected) in cases.iter().chain(cases.iter()).enumerate() {
            let pc_mode = index >= cases.len();
            cancel::check(stop)?;
            let token = (index + 1) as u16;
            // Exercise the real Raiju decoder; a separate gyro tag identifies delivery,
            // so an incorrect stick value cannot accidentally pass by timing out unseen.
            let mut raw = [0u8; 64];
            raw[0] = 1;
            raw[1..5].copy_from_slice(expected);
            raw[8] = 8;
            raw[32] = 128;
            raw[36] = 128;
            let mut state = protocol::parse_raiju(&raw).context("Sweep input failed to decode")?;
            if pc_mode {
                let signed = |b: u8| -> i16 {
                    if b >= 128 {
                        (((b as i32 - 128) * 32767 + 63) / 127) as i16
                    } else {
                        (-(((128 - b as i32) * 32768 + 64) / 128)) as i16
                    }
                };
                let reverse_y =
                    |b: u8| -> i16 { (-(signed(b) as i32)).clamp(-32768, 32767) as i16 };
                state = crate::input::translate(crate::input::Gamepad {
                    lx: signed(expected[0]),
                    ly: reverse_y(expected[1]),
                    rx: signed(expected[2]),
                    ry: reverse_y(expected[3]),
                    ..Default::default()
                });
            }
            state.gyro_x = token as i16;
            pad.set(state)?;
            let deadline = Instant::now() + Duration::from_secs(2);
            let actual = loop {
                cancel::check(stop)?;
                if Instant::now() >= deadline {
                    bail!(
                        "Stick vector {} was not delivered within 2 seconds",
                        index + 1
                    );
                }
                let count = input.read_timeout(&mut report, 50)?;
                if count == 64
                    && report[0] == 1
                    && u16::from_le_bytes([report[16], report[17]]) == token
                {
                    break axes(&report[..count]).unwrap();
                }
            };
            let error = expected
                .iter()
                .zip(actual)
                .map(|(&a, b)| a.abs_diff(b))
                .max()
                .unwrap();
            summary.tested += 1;
            summary.mismatches += usize::from(error != 0);
            summary.max_error = summary.max_error.max(error);
            writeln!(
                csv,
                "{token},{},{},{},{},{},{},{},{},{},{error}",
                if pc_mode { "PC" } else { "PS5" },
                expected[0],
                expected[1],
                expected[2],
                expected[3],
                actual[0],
                actual[1],
                actual[2],
                actual[3]
            )?;
            if summary.tested % 256 == 0 {
                log(format!(
                    "{} / {} vectors · {} changed",
                    summary.tested,
                    cases.len() * 2,
                    summary.mismatches
                ));
            }
        }
        Ok(())
    })();
    csv.flush()?;
    let status = match &result {
        Ok(()) => "complete",
        Err(e) if cancel::is_cancelled(e) => "cancelled",
        Err(_) => "failed",
    };
    fs::write(
        directory.join("stick-sweep.json"),
        serde_json::to_vec_pretty(&serde_json::json!({
            "recorded_at":chrono::Local::now(),"status":status,"result":summary,"error":result.as_ref().err().map(|e|format!("{e:#}")),
            "method":"Synthetic PS5-mode Raiju reports and PC-mode XInput states -> production translators -> VIIPER -> USBIP -> Windows HID. Independent gyro sequence tag identifies each report; four expected output axis bytes compared exactly.",
            "coverage":"For EACH mode: all 256 output values on each of four axes, 16 extrema combinations, opposing diagonal ramps, 512 seeded mixed vectors, neutral. PC inputs use signed 16-bit representatives; rounding to 8-bit is inherent to DualSense output. Not exhaustive over all combinations.",
            "scope":"Bridge value preservation, including center and endpoints. Does not measure physical stick precision, deadzones in the controller/game, circularity, or actuation latency. Game must be closed; gyro tag is diagnostic only."
        }))?,
    )?;
    result?;
    if summary.mismatches != 0 {
        bail!(
            "{} stick vectors changed; maximum error {} raw counts. Results saved.",
            summary.mismatches,
            summary.max_error
        );
    }
    Ok(summary)
}

#[derive(Clone, Debug, Serialize)]
pub struct StickSnapshot {
    pub input: [u8; 4],
    pub output: [u8; 4],
    pub minimum: [u8; 4],
    pub maximum: [u8; 4],
    pub distinct: [usize; 4],
    pub source_changes: u64,
    pub matched: u64,
    pub skipped: u64,
    pub unexpected: u64,
}
impl Default for StickSnapshot {
    fn default() -> Self {
        Self {
            input: [128; 4],
            output: [128; 4],
            minimum: [255; 4],
            maximum: [0; 4],
            distinct: [0; 4],
            source_changes: 0,
            matched: 0,
            skipped: 0,
            unexpected: 0,
        }
    }
}
pub struct StickMonitor {
    pub ready: bool,
    pub snapshot: StickSnapshot,
    pub reader_error: Option<String>,
    pending: VecDeque<([u8; 4], Instant)>,
    seen: [[bool; 256]; 4],
    origin: Instant,
    rows: BufWriter<File>,
    directory: std::path::PathBuf,
}
impl StickMonitor {
    pub fn new(directory: &Path) -> Result<Self> {
        fs::create_dir_all(directory)?;
        let mut rows = BufWriter::new(File::create(directory.join("stick-events.csv"))?);
        writeln!(rows, "elapsed_ms,event,lx,ly,rx,ry")?;
        Ok(Self {
            ready: false,
            snapshot: StickSnapshot::default(),
            reader_error: None,
            pending: VecDeque::with_capacity(256),
            seen: [[false; 256]; 4],
            origin: Instant::now(),
            rows,
            directory: directory.into(),
        })
    }
    fn row(&mut self, event: &str, a: [u8; 4], time: Instant) {
        if let Err(e) = writeln!(
            self.rows,
            "{:.3},{event},{},{},{},{}",
            time.duration_since(self.origin).as_secs_f64() * 1000.0,
            a[0],
            a[1],
            a[2],
            a[3]
        ) {
            self.reader_error = Some(e.to_string());
        }
    }
    pub fn source(&mut self, a: [u8; 4], time: Instant) {
        if !self.ready {
            self.snapshot.input = a;
            return;
        }
        for (axis, &v) in a.iter().enumerate() {
            self.snapshot.minimum[axis] = self.snapshot.minimum[axis].min(v);
            self.snapshot.maximum[axis] = self.snapshot.maximum[axis].max(v);
            if !self.seen[axis][v as usize] {
                self.seen[axis][v as usize] = true;
                self.snapshot.distinct[axis] += 1;
            }
        }
        if a == self.snapshot.input {
            return;
        }
        self.snapshot.input = a;
        self.snapshot.source_changes += 1;
        while self.pending.len() >= 512
            || self
                .pending
                .front()
                .is_some_and(|(_, t)| time.duration_since(*t) > Duration::from_secs(2))
        {
            self.pending.pop_front();
            self.snapshot.skipped += 1;
        }
        self.pending.push_back((a, time));
        self.row("source", a, time);
    }
    pub fn observe(&mut self, a: [u8; 4], time: Instant) {
        if !self.ready || a == self.snapshot.output {
            self.snapshot.output = a;
            return;
        }
        self.snapshot.output = a;
        if let Some(index) = self
            .pending
            .iter()
            .position(|(value, t)| *value == a && *t <= time)
        {
            self.pending.drain(..=index);
            self.snapshot.skipped += index as u64;
            self.snapshot.matched += 1;
            self.row("exact_match", a, time);
        } else {
            self.snapshot.unexpected += 1;
            self.row("unmatched_output", a, time);
        }
    }
    pub fn save(&mut self) -> Result<()> {
        self.rows.flush()?;
        fs::write(
            self.directory.join("stick-results.json"),
            serde_json::to_vec_pretty(&serde_json::json!({
                "recorded_at":chrono::Local::now(),"snapshot":self.snapshot,"pending_at_stop":self.pending.len(),"reader_error":self.reader_error,
                "method":"Exact four-axis output-grid tuples matched to preceding translated source changes. Heartbeats excluded. Earlier unobserved changes counted as skipped, not value errors. CSV records translated source and output changes; see input-source.json for the source mode.",
                "scope":"Observed preservation on the 0..255 output grid. Repeated tuples are not unique sequence IDs; do not infer latency or absolute physical accuracy from this test. Insufficient movement is not a full-range pass. Center is 128. PC mode includes 16-to-8-bit rounding and Y convention conversion; no additional deadzone or response curve is applied."
            }))?,
        )?;
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_axis_value_is_covered_and_decoded_exactly() {
        let cases = sweep_cases();
        assert_eq!(cases.len(), 1809);
        let mut seen = [[false; 256]; 4];
        for input in cases {
            let mut raw = [0; 64];
            raw[0] = 1;
            raw[8] = 8;
            raw[1..5].copy_from_slice(&input);
            let s = protocol::parse_raiju(&raw).unwrap();
            assert_eq!(
                [s.lx, s.ly, s.rx, s.ry].map(|x| (i16::from(x) + 128) as u8),
                input
            );
            for axis in 0..4 {
                seen[axis][input[axis] as usize] = true;
            }
        }
        assert!(seen.iter().flatten().all(|v| *v));
    }
}
