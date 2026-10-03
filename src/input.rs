use crate::{
    native,
    protocol::{self, PadState},
};
use anyhow::{Context, Result, bail};
use libloading::os::windows::Library;
use std::{
    ptr,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::Threading::*,
};

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct Gamepad {
    pub buttons: u16,
    pub l2: u8,
    pub r2: u8,
    pub lx: i16,
    pub ly: i16,
    pub rx: i16,
    pub ry: i16,
}
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct State {
    packet: u32,
    pad: Gamepad,
}
#[repr(C)]
#[derive(Default)]
struct CapsEx {
    kind: u8,
    subtype: u8,
    flags: u16,
    pad: Gamepad,
    vibration: [u16; 2],
    vid: u16,
    pid: u16,
    version: u16,
    unknown: u16,
    unknown2: u32,
}
type GetState = unsafe extern "system" fn(u32, *mut State) -> u32;
type GetCaps = unsafe extern "system" fn(u32, u32, u32, *mut CapsEx) -> u32;

// Preserve the center and both endpoints, rounding to the nearest DualSense
// byte value. Negative and positive half ranges have 128 and 127 output steps.
pub fn axis(v: i32) -> u8 {
    if v >= 0 {
        (128 + (v.min(32767) * 127 + 16383) / 32767) as u8
    } else {
        (128 - ((-v).min(32768) * 128 + 16384) / 32768) as u8
    }
}
pub fn translate(p: Gamepad) -> PadState {
    let raw = [
        axis(p.lx as i32),
        axis(-(p.ly as i32)),
        axis(p.rx as i32),
        axis(-(p.ry as i32)),
    ];
    let mut buttons = 0u32;
    for (source, target) in [
        (0x1000, 0x20),
        (0x2000, 0x40),
        (0x4000, 0x10),
        (0x8000, 0x80),
        (0x100, 0x100),
        (0x200, 0x200),
        (0x20, 0x1000),
        (0x10, 0x2000),
        (0x40, 0x4000),
        (0x80, 0x8000),
        (0x400, 0x10000),
    ] {
        if p.buttons & source != 0 {
            buttons |= target;
        }
    }
    if p.l2 > 0 {
        buttons |= 0x400;
    }
    if p.r2 > 0 {
        buttons |= 0x800;
    }
    PadState {
        lx: (raw[0] as i16 - 128) as i8,
        ly: (raw[1] as i16 - 128) as i8,
        rx: (raw[2] as i16 - 128) as i8,
        ry: (raw[3] as i16 - 128) as i8,
        buttons,
        dpad: (p.buttons & 15) as u8,
        l2: p.l2,
        r2: p.r2,
        ..Default::default()
    }
}
pub struct PcInput {
    touchpad: crate::touchpad::Touchpad,
    hiding: Option<Box<crate::hiding::Guard>>,
    precise: bool,
    _library: Library,
    get: GetState,
    caps: GetCaps,
    slot: u32,
    pid: u16,
    timer: HANDLE,
    next: Instant,
    identity_check: Instant,
    last_packet: Option<u32>,
}
impl PcInput {
    fn open() -> Result<Self> {
        let expected_pid = crate::hiding::prepare()?;
        // Resolve only Microsoft's system copy. The optional ordinals match the
        // ABI used by SDL's Windows backend; identification is mandatory.
        let library = unsafe { Library::load_with_flags("xinput1_4.dll", 0x800) }?;
        let caps: GetCaps = unsafe {
            *library
                .get_ordinal(108)
                .context("This Windows XInput library cannot identify controller device IDs")?
        };
        let get: GetState = unsafe {
            library
                .get_ordinal(100)
                .map(|s| *s)
                .or_else(|_| library.get(b"XInputGetState\0").map(|s| *s))?
        };
        let mut found = Vec::new();
        for slot in 0..4 {
            let mut c = CapsEx::default();
            if unsafe { caps(1, slot, 0, &mut c) } == 0
                && c.vid == 0x1532
                && matches!(c.pid, 0x1025 | 0x1027)
            {
                found.push((slot, c.pid));
            }
        }
        if found.is_empty() {
            return Err(native::SourceUnavailable.into());
        }
        if found.len() != 1 {
            bail!("Connect only one Raiju for this bridge");
        }
        if found[0].1 != expected_pid {
            bail!("Raiju changed during discovery; retry Start");
        }
        let touchpad = crate::touchpad::Touchpad::open(found[0].1)?;
        let timer = unsafe {
            CreateWaitableTimerExW(
                ptr::null(),
                ptr::null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                TIMER_ALL_ACCESS,
            )
        };
        if timer.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(Self {
            touchpad,
            hiding: None,
            precise: crate::desktop::Settings::load()
                .unwrap_or_default()
                .precise_pc,
            _library: library,
            get,
            caps,
            slot: found[0].0,
            pid: found[0].1,
            timer,
            next: Instant::now(),
            identity_check: Instant::now(),
            last_packet: None,
        })
    }
    fn read(&mut self) -> Result<Frame> {
        let now = Instant::now();
        if self.precise {
            // A waitable timer missed the 0.5 ms target on the tested PC. Yield
            // to other ready threads until the deadline; this costs CPU time.
            // It is opt-out in diagnostics and used only while PC input runs.
            while Instant::now() < self.next {
                std::thread::yield_now();
            }
        } else if now < self.next {
            let due = -((self.next.duration_since(now).as_nanos() / 100).max(1) as i64);
            if unsafe { SetWaitableTimer(self.timer, &due, 0, None, ptr::null(), 0) } == 0 {
                return Err(std::io::Error::last_os_error().into());
            }
            if unsafe { WaitForSingleObject(self.timer, 50) } != WAIT_OBJECT_0 {
                bail!("PC input timer did not signal");
            }
        }
        let now = Instant::now();
        self.next = if now.saturating_duration_since(self.next) > Duration::from_micros(500) {
            now + Duration::from_micros(500)
        } else {
            self.next + Duration::from_micros(500)
        };
        if now.duration_since(self.identity_check) > Duration::from_secs(1) {
            self.touchpad.check()?;
            if let Some(guard) = &mut self.hiding {
                guard.check()?;
            }
            let mut c = CapsEx::default();
            if unsafe { (self.caps)(1, self.slot, 0, &mut c) } != 0
                || c.vid != 0x1532
                || c.pid != self.pid
            {
                return Err(native::SourceUnavailable.into());
            }
            self.identity_check = now;
        }
        let mut raw = State::default();
        if unsafe { (self.get)(self.slot, &mut raw) } != 0 {
            return Err(native::SourceUnavailable.into());
        }
        let received = Instant::now();
        let mut fresh = self.last_packet != Some(raw.packet);
        self.last_packet = Some(raw.packet);
        let mut state = translate(raw.pad);
        fresh |= self.touchpad.apply(&mut state)?;
        Ok(Frame {
            state,
            received,
            fresh,
            raw_pc: Some([raw.pad.lx, raw.pad.ly, raw.pad.rx, raw.pad.ry]),
        })
    }
}
impl Drop for PcInput {
    fn drop(&mut self) {
        unsafe {
            CancelWaitableTimer(self.timer);
            CloseHandle(self.timer);
        }
    }
}
pub struct Frame {
    pub state: PadState,
    pub received: Instant,
    pub fresh: bool,
    pub raw_pc: Option<[i16; 4]>,
}
impl Frame {
    pub fn axes(&self) -> [u8; 4] {
        [self.state.lx, self.state.ly, self.state.rx, self.state.ry].map(|x| (x as i16 + 128) as u8)
    }
}
pub enum Source {
    Ps(hidapi::HidDevice),
    Pc(PcInput),
}
impl Source {
    pub fn begin_forwarding(&mut self) -> Result<()> {
        if let Self::Pc(input) = self {
            input.hiding = Some(Box::new(crate::hiding::Guard::start(input.pid)?));
            let mut state = State::default();
            if unsafe { (input.get)(input.slot, &mut state) } != 0 {
                bail!(
                    "HidHide cannot allow this running process. Quit and reopen the bridge after installing the driver, then retry Start."
                );
            }
            input.touchpad.begin_forwarding();
        }
        Ok(())
    }
    pub fn finish_forwarding(&mut self) -> Result<()> {
        if let Self::Pc(input) = self
            && let Some(mut guard) = input.hiding.take()
        {
            guard.finish()?;
        }
        Ok(())
    }
    pub fn set_precise(&mut self, precise: bool) {
        if let Self::Pc(input) = self {
            input.precise = precise;
        }
    }
    pub fn open() -> Result<(Self, String)> {
        let api = hidapi::HidApi::new()?;
        let pads: Vec<_> = api
            .device_list()
            .filter(|d| {
                d.vendor_id() == 0x1532
                    && matches!(d.product_id(), 0x1024..=0x1027)
                    && d.usage_page() == 1
                    && matches!(d.usage(), 4 | 5)
            })
            .collect();
        if pads.len() > 1 {
            bail!("Connect only one Raiju for this bridge");
        }
        if pads
            .first()
            .is_some_and(|d| matches!(d.product_id(), 0x1024 | 0x1026))
        {
            let (input, name) = native::source()?;
            Ok((Self::Ps(input), format!("PS5 · {name}")))
        } else {
            let input = PcInput::open()?;
            let name = format!("PC · 1532:{:04X} · 2,000 Hz target", input.pid);
            Ok((Self::Pc(input), name))
        }
    }
    pub fn is_pc(&self) -> bool {
        matches!(self, Self::Pc(_))
    }
    pub fn read(&mut self, buffer: &mut [u8]) -> Result<Option<Frame>> {
        match self {
            Self::Pc(input) => input.read().map(Some),
            Self::Ps(input) => {
                let n = input.read_timeout(buffer, 50)?;
                let received = Instant::now();
                if n == 0 {
                    return Ok(None);
                }
                let state =
                    protocol::parse_raiju(&buffer[..n]).context("Unsupported Raiju report")?;
                Ok(Some(Frame {
                    state,
                    received,
                    fresh: true,
                    raw_pc: None,
                }))
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn xinput_abi() {
        assert_eq!(std::mem::size_of::<Gamepad>(), 12);
        assert_eq!(std::mem::size_of::<State>(), 16);
        assert_eq!(std::mem::size_of::<CapsEx>(), 32);
        assert_eq!(std::mem::offset_of!(CapsEx, vid), 20);
    }
    #[test]
    fn entire_pc_range_is_monotonic_and_centered() {
        let mut last = 0;
        for v in -32768..=32767 {
            let a = axis(v);
            assert!(a >= last);
            last = a;
        }
        assert_eq!([axis(-32768), axis(0), axis(32767)], [0, 128, 255]);
    }
    #[test]
    fn directions_faces_and_independent_triggers() {
        let s = translate(Gamepad {
            buttons: 0x1001,
            l2: 123,
            r2: 231,
            lx: i16::MIN,
            ly: i16::MAX,
            rx: i16::MAX,
            ry: i16::MIN,
        });
        assert_eq!([s.lx, s.ly, s.rx, s.ry], [-128, -128, 127, 127]);
        assert_eq!(s.buttons, 0xc20);
        assert_eq!(s.dpad, 1);
        assert_eq!((s.l2, s.r2), (123, 231));
        assert_eq!(translate(Gamepad::default()), PadState::default());
    }
}
