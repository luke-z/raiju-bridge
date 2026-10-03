//! Reject incompatible backend descriptors before reporting a usable controller.
use anyhow::{Context, Result, bail};
use std::{
    ffi::CStr,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Devices::HumanInterfaceDevice::*,
    Foundation::INVALID_HANDLE_VALUE,
    Storage::FileSystem::{CreateFileA, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING},
};

struct Preparsed(PHIDP_PREPARSED_DATA);
impl Drop for Preparsed {
    fn drop(&mut self) {
        unsafe { HidD_FreePreparsedData(self.0) };
    }
}

fn report_sizes(input: u16, output: u16) -> Result<()> {
    if (input, output) != (64, 48) {
        bail!(
            "Incompatible virtual DualSense reports ({input}/{output} bytes; expected 64/48). Replace libVIIPER.dll with the one included in this app's ZIP and restart the app."
        );
    }
    Ok(())
}

pub(crate) fn check(path: &CStr) -> Result<()> {
    // Read descriptor metadata only; do not reserve or write to the HID device.
    let raw = unsafe {
        CreateFileA(
            path.as_ptr().cast(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            0,
            ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error())
            .context("Cannot inspect the virtual DualSense descriptor");
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut data = 0;
    if !unsafe { HidD_GetPreparsedData(handle.as_raw_handle(), &mut data) } {
        return Err(std::io::Error::last_os_error())
            .context("Cannot read the virtual DualSense descriptor");
    }
    let data = Preparsed(data);
    let mut caps = HIDP_CAPS::default();
    let status = unsafe { HidP_GetCaps(data.0, &mut caps) };
    if status != HIDP_STATUS_SUCCESS {
        bail!("Cannot decode the virtual DualSense descriptor: {status:#x}");
    }
    report_sizes(caps.InputReportByteLength, caps.OutputReportByteLength)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_upstream_report_size_that_native_games_cannot_open() {
        assert!(report_sizes(64, 48).is_ok());
        assert!(report_sizes(64, 64).is_err());
        assert!(report_sizes(48, 48).is_err());
    }
}
