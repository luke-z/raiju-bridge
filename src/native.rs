use crate::protocol::PadState;
use anyhow::{Context, Result, bail};
use hidapi::{DeviceInfo, HidApi, HidDevice};
use libloading::Library;
use std::{
    collections::HashSet,
    ffi::{CStr, CString, c_char, c_void},
    sync::{OnceLock, atomic::AtomicBool},
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
    System::Threading::{CreateSemaphoreW, ReleaseSemaphore, WaitForSingleObject},
};

#[repr(C)]
struct ServerConfig {
    address: *const c_char,
    connection_timeout: u64,
    device_timeout: u64,
    flush_interval: u32,
}
type LogFn = unsafe extern "C" fn(i32, *const c_char);
type NewServer = unsafe extern "C" fn(*const ServerConfig, *mut usize, LogFn) -> bool;
type CreateBus = unsafe extern "C" fn(usize, *mut u32) -> bool;
type CreateDevice =
    unsafe extern "C" fn(usize, *mut usize, u32, bool, u16, u16, *const c_void) -> bool;
type SetState = unsafe extern "C" fn(usize, PadState) -> bool;
type Remove = unsafe extern "C" fn(usize) -> bool;
type RemoveBus = unsafe extern "C" fn(usize, u32) -> bool;

struct Api {
    _library: Library,
    new_server: NewServer,
    create_bus: CreateBus,
    create_device: CreateDevice,
    set_state: SetState,
    remove: Remove,
    remove_bus: RemoveBus,
    close: Remove,
}
static API: OnceLock<Result<Api, String>> = OnceLock::new();

unsafe extern "C" fn native_log(level: i32, message: *const c_char) {
    if level >= 4 && !message.is_null() {
        eprintln!(
            "VIIPER: {}",
            unsafe { CStr::from_ptr(message) }.to_string_lossy()
        );
    }
}

impl Api {
    fn load() -> Result<Self> {
        let path = std::env::current_exe()?
            .parent()
            .context("Missing executable directory")?
            .join("libVIIPER.dll");
        // Pin the Go DLL for the process lifetime: unloading it could invalidate Go runtime threads.
        let library = unsafe { Library::new(&path) }.with_context(|| {
            format!(
                "Cannot load {}. Keep libVIIPER.dll beside the app.",
                path.display()
            )
        })?;
        unsafe {
            Ok(Self {
                new_server: *library.get(b"NewUSBServer\0")?,
                create_bus: *library.get(b"CreateUSBBus\0")?,
                create_device: *library.get(b"CreateDualSenseDevice\0")?,
                set_state: *library.get(b"SetDualSenseDeviceState\0")?,
                remove: *library.get(b"RemoveDualSenseDevice\0")?,
                remove_bus: *library.get(b"RemoveUSBBus\0")?,
                close: *library.get(b"CloseUSBServer\0")?,
                _library: library,
            })
        }
    }
    fn get() -> Result<&'static Self> {
        API.get_or_init(|| Self::load().map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.clone()))
    }
}

pub struct ExclusiveSession(HANDLE);
impl ExclusiveSession {
    pub fn acquire() -> Result<Self> {
        let name: Vec<u16> = "Local\\RaijuBridgeSingleInstance\0"
            .encode_utf16()
            .collect();
        let handle = unsafe { CreateSemaphoreW(std::ptr::null(), 1, 1, name.as_ptr()) };
        if handle.is_null() {
            return Err(std::io::Error::last_os_error().into());
        }
        if unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0 {
            unsafe {
                CloseHandle(handle);
            }
            bail!("Another bridge or test is running. Stop it first.");
        }
        Ok(Self(handle))
    }
}
impl Drop for ExclusiveSession {
    fn drop(&mut self) {
        unsafe {
            ReleaseSemaphore(self.0, 1, std::ptr::null_mut());
            CloseHandle(self.0);
        }
    }
}

pub struct VirtualPad {
    api: &'static Api,
    server: usize,
    device: usize,
    bus: u32,
}
impl VirtualPad {
    pub fn attach(flush_interval: u32, stop: &AtomicBool) -> Result<Self> {
        let api = Api::get()?;
        let mut pad = Self {
            api,
            server: 0,
            device: 0,
            bus: 0,
        };
        let address = CString::new("localhost:0")?;
        let config = ServerConfig {
            address: address.as_ptr(),
            connection_timeout: 0,
            device_timeout: 0,
            flush_interval,
        };
        unsafe {
            if !(api.new_server)(&config, &mut pad.server, native_log) {
                bail!("Could not start the local USB backend.");
            }
            let mut bus = 0;
            if !(api.create_bus)(pad.server, &mut bus) {
                bail!("Could not create the virtual USB bus.");
            }
            pad.bus = bus;
            // Go's exported cgo call stays on this OS thread. usbip-win2
            // supports cancellation of its synchronous attach IOCTL.
            if !crate::cancel::synchronous_io(stop, || {
                (api.create_device)(
                    pad.server,
                    &mut pad.device,
                    bus,
                    true,
                    0,
                    0,
                    std::ptr::null(),
                )
            })? {
                bail!(
                    "Could not attach DualSense. Check that the USBip driver is installed and finish its requested reboot."
                );
            }
        }
        pad.set(PadState::default())?;
        Ok(pad)
    }
    #[inline]
    pub fn set(&self, state: PadState) -> Result<()> {
        if unsafe { (self.api.set_state)(self.device, state) } {
            Ok(())
        } else {
            bail!("Virtual DualSense state update failed.")
        }
    }
}
impl Drop for VirtualPad {
    fn drop(&mut self) {
        unsafe {
            if self.device != 0 {
                (self.api.set_state)(self.device, PadState::default());
                (self.api.remove)(self.device);
            }
            if self.server != 0 {
                // CloseUSBServer closes its listener but does not release the global
                // bus allocation in VIIPER 0.8.2. Explicit bus removal is required
                // for repeated starts in the same process, including early cancellation.
                if self.bus != 0 {
                    (self.api.remove_bus)(self.server, self.bus);
                }
                (self.api.close)(self.server);
            }
        }
    }
}

fn gamepad(info: &DeviceInfo) -> bool {
    info.usage_page() == 1 && matches!(info.usage(), 4 | 5)
}

#[derive(Debug)]
pub struct SourceUnavailable;
impl std::fmt::Display for SourceUnavailable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Waiting for Raiju · connect it in PC or PS5 mode")
    }
}
impl std::error::Error for SourceUnavailable {}

pub fn source() -> Result<(HidDevice, String)> {
    let api = HidApi::new()?;
    let devices: Vec<_> = api
        .device_list()
        .filter(|d| {
            d.vendor_id() == 0x1532 && matches!(d.product_id(), 0x1024 | 0x1026) && gamepad(d)
        })
        .collect();
    if devices.is_empty() {
        return Err(SourceUnavailable.into());
    }
    if devices.len() != 1 {
        bail!("Connect only one Raiju for this bridge.");
    }
    let info = devices[0];
    Ok((
        info.open_device(&api)?,
        format!(
            "1532:{:04X} · {}",
            info.product_id(),
            if info.product_id() == 0x1024 {
                "USB"
            } else {
                "dongle"
            }
        ),
    ))
}

pub fn sony_paths() -> Result<HashSet<Vec<u8>>> {
    let api = HidApi::new()?;
    Ok(api
        .device_list()
        .filter(|d| d.vendor_id() == 0x054c && d.product_id() == 0x0ce6 && gamepad(d))
        .map(|d| d.path().to_bytes().to_vec())
        .collect())
}

pub fn wait_for_virtual(previous: &HashSet<Vec<u8>>, stop: &AtomicBool) -> Result<HidDevice> {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        crate::cancel::check(stop)?;
        let api = HidApi::new()?;
        let mut candidates = api.device_list().filter(|d| {
            d.vendor_id() == 0x054c
                && d.product_id() == 0x0ce6
                && gamepad(d)
                && !previous.contains(d.path().to_bytes())
        });
        if let Some(info) = candidates.next() {
            crate::cancel::check(stop)?;
            if candidates.next().is_some() {
                bail!("More than one new DualSense appeared; cannot identify this session safely.");
            }
            return info.open_device(&api).map_err(Into::into);
        }
        crate::cancel::pause(stop, Duration::from_millis(100))?;
    }
    crate::cancel::check(stop)?;
    bail!("Windows did not recognize the virtual DualSense within 15 seconds.")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn server_abi() {
        assert_eq!(std::mem::size_of::<ServerConfig>(), 32);
        assert_eq!(std::mem::offset_of!(ServerConfig, flush_interval), 24);
    }
}
