use anyhow::{Result, bail};
use std::ptr;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::*;

pub(super) struct Selection {
    pub pid: u16,
    pub ids: Vec<String>,
}
fn present() -> Result<Vec<String>> {
    for _ in 0..3 {
        let mut size = 0;
        let code = unsafe {
            CM_Get_Device_ID_List_SizeW(&mut size, ptr::null(), CM_GETIDLIST_FILTER_PRESENT)
        };
        if code != CR_SUCCESS || size > 1048576 {
            bail!("Cannot enumerate connected controller devices ({code:#x})");
        }
        let mut buffer = vec![0; size as usize];
        let code = unsafe {
            CM_Get_Device_ID_ListW(
                ptr::null(),
                buffer.as_mut_ptr(),
                size,
                CM_GETIDLIST_FILTER_PRESENT,
            )
        };
        if code == CR_BUFFER_SMALL {
            continue;
        }
        if code != CR_SUCCESS {
            bail!("Cannot read connected controller devices ({code:#x})");
        }
        return buffer
            .split(|&v| v == 0)
            .filter(|s| !s.is_empty())
            .map(|s| {
                String::from_utf16(s)
                    .map(|s| s.to_uppercase())
                    .map_err(Into::into)
            })
            .collect();
    }
    bail!("Controller list changed during discovery; reconnect the Raiju and retry")
}
fn is_gamepad(id: &str, pid: u16, hid: bool) -> bool {
    let parts: Vec<_> = id.split('\\').collect();
    if parts.len() != 3 || parts[2].is_empty() {
        return false;
    }
    if hid {
        let prefix = format!("VID_1532&PID_{pid:04X}&IG_");
        parts[0] == "HID"
            && parts[1]
                .strip_prefix(&prefix)
                .is_some_and(|s| s.len() == 2 && u8::from_str_radix(s, 16).is_ok())
    } else {
        parts[0] == "USB" && parts[1] == format!("VID_1532&PID_{pid:04X}&MI_00")
    }
}
pub(super) fn valid(id: &str) -> bool {
    [0x1025, 0x1027]
        .iter()
        .any(|&pid| is_gamepad(id, pid, true) || is_gamepad(id, pid, false))
}
fn select(all: &[String]) -> Result<Option<Selection>> {
    let mut choices = Vec::new();
    for pid in [0x1025, 0x1027] {
        let usb: Vec<_> = all
            .iter()
            .filter(|id| is_gamepad(id, pid, false))
            .cloned()
            .collect();
        if usb.is_empty() {
            continue;
        }
        let hid: Vec<_> = all
            .iter()
            .filter(|id| is_gamepad(id, pid, true))
            .cloned()
            .collect();
        if usb.len() != 1 || hid.len() != 1 {
            bail!(
                "Cannot identify a single Raiju PC gamepad to hide. Connect only one Raiju and retry."
            );
        }
        choices.push(Selection {
            pid,
            ids: usb.into_iter().chain(hid).collect(),
        });
    }
    if choices.len() > 1 {
        bail!("Connect only one Raiju for this bridge");
    }
    Ok(choices.pop())
}
pub(super) fn find() -> Result<Option<Selection>> {
    select(&present()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn select_only_pc_gamepad_collections_and_current_slot() {
        let ids = [
            r"USB\VID_1532&PID_1025&MI_00\SOURCE",
            r"HID\VID_1532&PID_1025&IG_01\SOURCE",
            r"HID\VID_1532&PID_1025&MI_05&COL01\TOUCHPAD",
            r"HID\VID_1532&PID_1025&MI_01&COL03\MOUSE",
            r"USB\VID_1532&PID_1025\COMPOSITE",
            r"USB\VID_1532&PID_1024&MI_00\PS_AUDIO",
            r"HID\VID_054C&PID_0CE6\SONY",
        ]
        .map(String::from);
        assert_eq!(select(&ids).unwrap().unwrap().ids, ids[..2]);
        for id in &ids[2..] {
            assert!(!valid(id));
        }
        assert!(select(&ids[..1]).is_err());
        assert!(select(&ids[2..]).unwrap().is_none());
    }
    #[test]
    fn ambiguous_devices_are_never_hidden() {
        let ids = [
            r"USB\VID_1532&PID_1025&MI_00\ONE",
            r"USB\VID_1532&PID_1025&MI_00\TWO",
            r"HID\VID_1532&PID_1025&IG_00\ONE",
        ]
        .map(String::from);
        assert!(select(&ids).is_err());
    }
}
