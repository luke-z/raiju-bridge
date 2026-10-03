//! Optional, bounded live diagnostics. No recording or UI work on the input thread.
use crate::{
    accuracy::StickSnapshot,
    input::Frame,
    measure::{self, Stats},
    protocol,
    worker::Message,
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    ffi::CString,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, SyncSender},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub input_touch: crate::touchpad::State,
    pub output_touch: crate::touchpad::State,
    pub touch_changes: u64,
    pub touch_matches: u64,
    pub touch_skipped: u64,
    pub touch_unmatched: u64,
    pub pending_touch: usize,
    pub sticks: StickSnapshot,
    pub input_buttons: u32,
    pub highlight_buttons: u32,
    pub button_presses: u64,
    pub output_buttons: u32,
    pub input_triggers: [u8; 2],
    pub output_triggers: [u8; 2],
    pub button_matches: u64,
    pub button_skipped: u64,
    pub button_unmatched: u64,
    pub pending_buttons: usize,
    pub pending_sticks: usize,
    pub latency: Stats,
    pub last_button: String,
    pub source_queue_dropped: u64,
    pub reader_error: Option<String>,
}
struct Capture {
    touch: crate::touchpad::State,
    axes: [u8; 4],
    buttons: u32,
    triggers: [u8; 2],
    received: Instant,
}
struct Model {
    touch: VecDeque<(crate::touchpad::State, Instant)>,
    snapshot: Snapshot,
    ready_at: Instant,
    buttons: VecDeque<(u32, Instant)>,
    sticks: VecDeque<([u8; 4], Instant)>,
    delays: VecDeque<f64>,
    seen: [[bool; 256]; 4],
    flashes: [Option<Instant>; 32],
    stats_updated: Instant,
    stats_dirty: bool,
}
impl Model {
    fn new(ready_at: Instant) -> Self {
        Self {
            touch: VecDeque::with_capacity(256),
            snapshot: Snapshot::default(),
            ready_at,
            buttons: VecDeque::with_capacity(256),
            sticks: VecDeque::with_capacity(512),
            delays: VecDeque::with_capacity(4096),
            seen: [[false; 256]; 4],
            flashes: [None; 32],
            stats_updated: Instant::now(),
            stats_dirty: false,
        }
    }
    fn source(&mut self, capture: Capture) {
        let s = &mut self.snapshot;
        if capture.received >= self.ready_at {
            if capture.touch != s.input_touch {
                if self.touch.len() >= 256 {
                    self.touch.pop_front();
                    s.touch_skipped += 1;
                }
                self.touch.push_back((capture.touch, capture.received));
                s.touch_changes += 1;
            }
            let pressed = capture.buttons & !s.input_buttons;
            s.button_presses += pressed.count_ones() as u64;
            for (bit, flash) in self.flashes.iter_mut().enumerate() {
                if pressed & (1 << bit) != 0 {
                    *flash = Some(capture.received + Duration::from_millis(80));
                }
            }
            for (axis, &v) in capture.axes.iter().enumerate() {
                s.sticks.minimum[axis] = s.sticks.minimum[axis].min(v);
                s.sticks.maximum[axis] = s.sticks.maximum[axis].max(v);
                if !self.seen[axis][v as usize] {
                    self.seen[axis][v as usize] = true;
                    s.sticks.distinct[axis] += 1;
                }
            }
            if capture.axes != s.sticks.input {
                while self.sticks.len() >= 512
                    || self.sticks.front().is_some_and(|(_, t)| {
                        capture.received.saturating_duration_since(*t) > Duration::from_secs(2)
                    })
                {
                    self.sticks.pop_front();
                    s.sticks.skipped += 1;
                }
                self.sticks.push_back((capture.axes, capture.received));
                s.sticks.source_changes += 1;
            }
            if capture.buttons != s.input_buttons {
                while self.buttons.len() >= 256
                    || self.buttons.front().is_some_and(|(_, t)| {
                        capture.received.saturating_duration_since(*t) > Duration::from_secs(2)
                    })
                {
                    self.buttons.pop_front();
                    s.button_skipped += 1;
                }
                self.buttons.push_back((capture.buttons, capture.received));
            }
        }
        s.sticks.input = capture.axes;
        s.input_buttons = capture.buttons;
        s.input_triggers = capture.triggers;
        s.input_touch = capture.touch;
    }
    fn touch_output(&mut self, touch: crate::touchpad::State, arrived: Instant) {
        let s = &mut self.snapshot;
        if touch != s.output_touch && arrived >= self.ready_at {
            if let Some(i) = self
                .touch
                .iter()
                .position(|(t, at)| *t == touch && *at <= arrived)
            {
                self.touch.drain(..=i);
                s.touch_skipped += i as u64;
                s.touch_matches += 1;
            } else {
                s.touch_unmatched += 1;
            }
        }
        s.output_touch = touch;
    }
    fn output(&mut self, axes: [u8; 4], buttons: u32, triggers: [u8; 2], arrived: Instant) {
        let s = &mut self.snapshot;
        if arrived >= self.ready_at {
            if axes != s.sticks.output {
                if let Some(index) = self
                    .sticks
                    .iter()
                    .position(|(a, t)| *a == axes && *t <= arrived)
                {
                    self.sticks.drain(..=index);
                    s.sticks.skipped += index as u64;
                    s.sticks.matched += 1;
                } else {
                    s.sticks.unexpected += 1;
                }
            }
            if buttons != s.output_buttons {
                if let Some(index) = self
                    .buttons
                    .iter()
                    .position(|(b, t)| *b == buttons && *t <= arrived)
                {
                    let last = self
                        .buttons
                        .iter()
                        .rposition(|(b, t)| *b == buttons && *t <= arrived)
                        .unwrap();
                    if index != last {
                        self.buttons.drain(..=last);
                        s.button_skipped += (last + 1) as u64;
                    } else {
                        self.buttons.drain(..index);
                        s.button_skipped += index as u64;
                        let (_, sent) = self.buttons.pop_front().unwrap();
                        let ms = arrived.duration_since(sent).as_secs_f64() * 1000.0;
                        if ms <= 2000.0 {
                            if self.delays.len() == 4096 {
                                self.delays.pop_front();
                            }
                            self.delays.push_back(ms);
                            self.stats_dirty = true;
                            s.button_matches += 1;
                            s.last_button = format!(
                                "{} · {ms:.3} ms",
                                if buttons == 0 {
                                    "Released".into()
                                } else {
                                    protocol::button_names(buttons)
                                }
                            );
                        } else {
                            s.button_skipped += 1;
                        }
                    }
                } else {
                    s.button_unmatched += 1;
                }
            }
        }
        s.sticks.output = axes;
        s.output_buttons = buttons;
        s.output_triggers = triggers;
    }
    fn snapshot(&mut self, dropped: u64) -> Snapshot {
        self.snapshot_at(dropped, Instant::now())
    }
    fn snapshot_at(&mut self, dropped: u64, now: Instant) -> Snapshot {
        if self.stats_dirty
            && (self.snapshot.latency.count == 0
                || now.saturating_duration_since(self.stats_updated) >= Duration::from_millis(100))
        {
            self.snapshot.latency = measure::stats(self.delays.iter().copied());
            self.stats_updated = now;
            self.stats_dirty = false;
        }
        self.snapshot.highlight_buttons = self.snapshot.input_buttons;
        for (bit, flash) in self.flashes.iter().enumerate() {
            if flash.is_some_and(|until| until > now) {
                self.snapshot.highlight_buttons |= 1 << bit;
            }
        }
        self.snapshot.source_queue_dropped = dropped;
        self.snapshot.pending_buttons = self.buttons.len();
        self.snapshot.pending_sticks = self.sticks.len();
        self.snapshot.pending_touch = self.touch.len();
        self.snapshot.clone()
    }
}

pub struct Live {
    source: SyncSender<Capture>,
    dropped: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<Snapshot>>,
}
impl Live {
    pub fn start(path: CString, updates: SyncSender<Message>) -> Self {
        let (source, rx) = mpsc::sync_channel(256);
        let stop = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicU64::new(0));
        let flag = stop.clone();
        let lost = dropped.clone();
        let thread = thread::spawn(move || {
            let mut model = Model::new(Instant::now() + Duration::from_millis(250));
            let run = (|| -> anyhow::Result<()> {
                let api = hidapi::HidApi::new()?;
                let input = api.open_path(&path)?;
                let mut report = [0u8; 128];
                let mut updated = Instant::now();
                while !flag.load(Ordering::Acquire) {
                    let n = input.read_timeout(&mut report, 25)?;
                    let arrived = Instant::now();
                    // Source samples enter this queue before submission. Drain after
                    // the HID read so a fast response cannot overtake its source.
                    for capture in rx.try_iter() {
                        model.source(capture);
                    }
                    if n > 0 {
                        let buttons = protocol::virtual_signature(&report[..n])
                            .ok_or_else(|| anyhow::anyhow!("Unsupported virtual report"))?;
                        let axes = crate::accuracy::axes(&report[..n]).unwrap();
                        model.output(axes, buttons, [report[5], report[6]], arrived);
                        model.touch_output(
                            crate::touchpad::State::from_virtual(&report[..n]).unwrap(),
                            arrived,
                        );
                    }
                    if updated.elapsed() >= Duration::from_millis(16) {
                        let _ = updates.try_send(Message::Diagnostics(Box::new(
                            model.snapshot(lost.load(Ordering::Relaxed)),
                        )));
                        updated = Instant::now();
                    }
                }
                Ok(())
            })();
            if let Err(error) = run {
                model.snapshot.reader_error = Some(format!("{error:#}"));
            }
            let snapshot = model.snapshot(lost.load(Ordering::Relaxed));
            let _ = updates.try_send(Message::Diagnostics(Box::new(snapshot.clone())));
            snapshot
        });
        Self {
            source,
            dropped,
            stop,
            thread: Some(thread),
        }
    }
    pub fn capture(&self, frame: &Frame) {
        if self
            .source
            .try_send(Capture {
                touch: crate::touchpad::State::from_pad(&frame.state),
                axes: frame.axes(),
                buttons: protocol::signature(&frame.state),
                triggers: [frame.state.l2, frame.state.r2],
                received: frame.received,
            })
            .is_err()
        {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
    pub fn finish(mut self) -> Snapshot {
        self.stop();
        self.thread
            .take()
            .unwrap()
            .join()
            .unwrap_or_else(|_| Snapshot {
                reader_error: Some("Diagnostic reader stopped unexpectedly".into()),
                ..Default::default()
            })
    }
}
impl Drop for Live {
    fn drop(&mut self) {
        self.stop();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn capture(time: Instant, buttons: u32, axes: [u8; 4]) -> Capture {
        Capture {
            touch: crate::touchpad::State::default(),
            received: time,
            buttons,
            axes,
            triggers: [0, 0],
        }
    }
    #[test]
    fn quick_tap_remains_visible_without_holding_the_input() {
        let t = Instant::now();
        let mut m = Model::new(t);
        m.source(capture(t, 0x20, [128; 4]));
        m.source(capture(t + Duration::from_millis(1), 0, [128; 4]));
        let s = m.snapshot_at(0, t + Duration::from_millis(16));
        assert_eq!(s.input_buttons, 0);
        assert_eq!(s.highlight_buttons, 0x20);
        assert_eq!(s.button_presses, 1);
        assert_eq!(
            m.snapshot_at(0, t + Duration::from_millis(81))
                .highlight_buttons,
            0
        );
    }
    #[test]
    fn simultaneous_buttons_and_sticks_are_observed() {
        let t = Instant::now();
        let mut m = Model::new(t);
        m.source(capture(t, 0x20, [0, 255, 64, 192]));
        m.output(
            [0, 255, 64, 192],
            0x20,
            [0, 0],
            t + Duration::from_micros(80),
        );
        let s = m.snapshot(0);
        assert_eq!((s.button_matches, s.sticks.matched), (1, 1));
        assert_eq!(s.latency.median_ms, Some(0.08));
        assert_eq!((s.pending_buttons, s.pending_sticks), (0, 0));
    }
    #[test]
    fn ambiguous_edges_do_not_become_latency_samples() {
        let t = Instant::now();
        let mut m = Model::new(t);
        for b in [0x20, 0, 0x20] {
            m.source(capture(t, b, [128; 4]));
        }
        m.output([128; 4], 0x20, [0, 0], t + Duration::from_micros(80));
        let s = m.snapshot(0);
        assert_eq!(s.button_skipped, 3);
        assert_eq!(s.latency.count, 0);
    }
    #[test]
    fn long_sessions_have_bounded_storage() {
        let t = Instant::now();
        let mut m = Model::new(t);
        for i in 0..10_000 {
            let b = if i % 2 == 0 { 0x20 } else { 0 };
            m.source(capture(t, b, [i as u8; 4]));
            m.output([i as u8; 4], b, [0, 0], t + Duration::from_micros(80));
        }
        assert_eq!(m.delays.len(), 4096);
        assert_eq!(m.snapshot(0).button_matches, 10_000);
        for i in 0..10_000 {
            m.source(capture(t, if i % 2 == 0 { 0x20 } else { 0 }, [i as u8; 4]));
        }
        assert!(m.buttons.len() <= 256 && m.sticks.len() <= 512);
    }
}
