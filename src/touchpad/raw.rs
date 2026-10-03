use super::{Decoder, Shared};
use anyhow::{Context, Result, bail};
use std::{
    cell::RefCell,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*,
    UI::{Input::*, WindowsAndMessaging::*},
};

pub(super) fn only_raiju(pid: u16) -> Result<String> {
    let api = hidapi::HidApi::new()?;
    let pads: Vec<_> = api
        .device_list()
        .filter(|d| d.usage_page() == 0xd && d.usage() == 5)
        .collect();
    if pads.len() != 1 || pads[0].vendor_id() != 0x1532 || pads[0].product_id() != pid {
        bail!(
            "PC touchpad forwarding requires the Raiju to be the only Precision Touchpad. Disconnect other touchpads or use PS5 mode."
        );
    }
    Ok(pads[0].path().to_string_lossy().to_uppercase())
}
struct ContextData {
    path: String,
    shared: Arc<Mutex<Shared>>,
    decoder: Decoder,
    last: super::State,
}
thread_local! { static CONTEXT:RefCell<Option<ContextData>>=const{RefCell::new(None)}; }
fn device_name(handle: HANDLE) -> Result<String> {
    let mut name = [0u16; 512];
    let mut len = name.len() as u32;
    if unsafe {
        GetRawInputDeviceInfoW(handle, RIDI_DEVICENAME, name.as_mut_ptr().cast(), &mut len)
    } == u32::MAX
    {
        bail!("Cannot identify raw touchpad");
    }
    Ok(
        String::from_utf16_lossy(&name[..(len as usize).min(name.len())])
            .trim_end_matches('\0')
            .to_uppercase(),
    )
}
fn receive(l: LPARAM, ctx: &mut ContextData) -> Result<()> {
    // A u64 buffer provides the alignment required by RAWINPUT on x64.
    let mut buffer = [0u64; 256];
    let mut bytes = size_of_val(&buffer) as u32;
    let n = unsafe {
        GetRawInputData(
            l as _,
            RID_INPUT,
            buffer.as_mut_ptr().cast(),
            &mut bytes,
            size_of::<RAWINPUTHEADER>() as u32,
        )
    };
    let offset = size_of::<RAWINPUTHEADER>() + 8;
    if n == u32::MAX || (n as usize) < offset {
        bail!("Cannot read raw touchpad packet");
    }
    let raw = unsafe { &*buffer.as_ptr().cast::<RAWINPUT>() };
    if raw.header.dwType != RIM_TYPEHID || device_name(raw.header.hDevice)? != ctx.path {
        return Ok(());
    }
    let hid = unsafe { raw.data.hid };
    let length = (hid.dwSizeHid as usize)
        .checked_mul(hid.dwCount as usize)
        .context("Invalid touchpad size")?;
    if hid.dwSizeHid != 20 || length > n as usize - offset {
        bail!("Unsupported Raiju PC touchpad report");
    }
    let data =
        unsafe { std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>().add(offset), length) };
    for report in data.as_chunks::<20>().0 {
        let state = ctx
            .decoder
            .decode(report)
            .context("Invalid Raiju touchpad contacts")?;
        if state != ctx.last {
            let mut shared = ctx.shared.lock().unwrap();
            // Device attachment can take seconds. Until forwarding is ready,
            // keep the latest position instead of replaying setup-time gestures.
            if !shared.forwarding {
                shared.pending.clear();
            }
            if shared.pending.len() >= 128 {
                bail!("Touchpad input queue overflow; restart the bridge");
            }
            shared.pending.push_back(state);
            ctx.last = state;
        }
    }
    Ok(())
}
unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_INPUT {
        CONTEXT.with(|cell| {
            if let Some(ctx) = cell.borrow_mut().as_mut()
                && let Err(e) = receive(l, ctx)
            {
                ctx.shared.lock().unwrap().error = Some(format!("{e:#}"));
            }
        });
    }
    unsafe { DefWindowProcW(hwnd, msg, w, l) }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            DestroyWindow(self.0);
        }
    }
}
fn create_window() -> Result<Window> {
    let class: Vec<u16> = "RaijuBridgeTouchpad\0".encode_utf16().collect();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(wndproc),
        lpszClassName: class.as_ptr(),
        ..Default::default()
    };
    // Reusing the class after a reconnect is expected.
    unsafe {
        RegisterClassW(&wc);
    }
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class.as_ptr(),
            class.as_ptr(),
            0,
            0,
            0,
            0,
            0,
            HWND_MESSAGE,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        )
    };
    if hwnd.is_null() {
        return Err(std::io::Error::last_os_error().into());
    }
    let window = Window(hwnd);
    let rid = RAWINPUTDEVICE {
        usUsagePage: 0xd,
        usUsage: 5,
        dwFlags: RIDEV_INPUTSINK,
        hwndTarget: hwnd,
    };
    if unsafe { RegisterRawInputDevices(&rid, 1, size_of::<RAWINPUTDEVICE>() as u32) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(window)
}
pub(super) struct Reader {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl Reader {
    pub fn start(path: String, shared: Arc<Mutex<Shared>>) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        let thread = thread::spawn(move || {
            CONTEXT.with(|cell| {
                *cell.borrow_mut() = Some(ContextData {
                    path,
                    shared,
                    decoder: Decoder::default(),
                    last: super::State::default(),
                })
            });
            let window = match create_window() {
                Ok(w) => w,
                Err(e) => {
                    let _ = tx.send(Err(format!("{e:#}")));
                    return;
                }
            };
            let _ = tx.send(Ok(()));
            while !flag.load(Ordering::Acquire) {
                unsafe {
                    MsgWaitForMultipleObjectsEx(
                        0,
                        ptr::null(),
                        20,
                        QS_ALLINPUT,
                        MWMO_INPUTAVAILABLE,
                    );
                    let mut msg = MSG::default();
                    while PeekMessageW(&mut msg, ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
                        TranslateMessage(&msg);
                        DispatchMessageW(&msg);
                    }
                }
            }
            drop(window);
            CONTEXT.with(|cell| *cell.borrow_mut() = None);
        });
        let reader = Self {
            stop,
            thread: Some(thread),
        };
        rx.recv_timeout(Duration::from_secs(5))
            .context("Touchpad reader did not start")?
            .map_err(anyhow::Error::msg)?;
        Ok(reader)
    }
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
