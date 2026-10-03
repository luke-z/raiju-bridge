//! HidHide's public 1.5 IOCTL protocol; no registry or driver installation edits.
use crate::desktop::wide;
use anyhow::{Context, Result, bail};
use std::{
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    ptr,
};
use windows_sys::Win32::{
    Foundation::{GENERIC_READ, INVALID_HANDLE_VALUE},
    Storage::FileSystem::*,
    System::{
        IO::DeviceIoControl, ProcessStatus::GetProcessImageFileNameW, Threading::GetCurrentProcess,
    },
};

pub(super) struct Driver(OwnedHandle);
impl Driver {
    pub fn open() -> Result<Self> {
        let handle = unsafe {
            CreateFileW(
                wide(r"\\.\HidHide").as_ptr(),
                GENERIC_READ,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(std::io::Error::last_os_error()).context(
                "Cannot access HidHide. PC mode needs its installed driver. Close the HidHide configuration window and finish any requested restart, or use PS5 mode");
        }
        Ok(Self(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }
    fn control(&self, function: u32, input: &[u8], output: &mut [u8]) -> Result<usize> {
        let mut returned = 0;
        let code = (32769 << 16) | (1 << 14) | (function << 2);
        let ok = unsafe {
            DeviceIoControl(
                self.0.as_raw_handle(),
                code,
                if input.is_empty() {
                    ptr::null()
                } else {
                    input.as_ptr().cast()
                },
                input.len() as u32,
                if output.is_empty() {
                    ptr::null_mut()
                } else {
                    output.as_mut_ptr().cast()
                },
                output.len() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error()).context("HidHide request failed");
        }
        Ok(returned as usize)
    }
    pub fn flag(&self, function: u32) -> Result<bool> {
        let mut value = [0];
        if self.control(function, &[], &mut value)? != 1 {
            bail!("Invalid HidHide flag response");
        }
        Ok(value[0] != 0)
    }
    pub fn set_flag(&self, function: u32, value: bool) -> Result<()> {
        self.control(function, &[u8::from(value)], &mut [])?;
        Ok(())
    }
    pub fn list(&self, function: u32) -> Result<Vec<String>> {
        let bytes = self.control(function, &[], &mut [])?;
        if bytes > 131072 || bytes % 2 != 0 {
            bail!("Invalid HidHide list size");
        }
        let mut buffer = vec![0; bytes];
        let written = self.control(function, &[], &mut buffer)?;
        if written > bytes || written % 2 != 0 {
            bail!("Invalid HidHide list response");
        }
        let words: Vec<u16> = buffer[..written]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|b| u16::from_le_bytes([b[0], b[1]]))
            .collect();
        words
            .split(|&x| x == 0)
            .filter(|s| !s.is_empty())
            .map(|s| String::from_utf16(s).map_err(Into::into))
            .collect()
    }
    pub fn set_list(&self, function: u32, list: &[String]) -> Result<()> {
        let mut words: Vec<u16> = list.iter().flat_map(|s| wide(s)).collect();
        if words.is_empty() {
            words.push(0);
        }
        words.push(0);
        let bytes: Vec<u8> = words.into_iter().flat_map(u16::to_le_bytes).collect();
        self.control(function, &bytes, &mut [])?;
        Ok(())
    }
    pub fn allow_current(&self) -> Result<()> {
        if self.flag(2054)? {
            bail!(
                "HidHide uses an inverse application list. Turn that option off before using PC mode."
            );
        }
        let mut path = vec![0; 32768];
        let n = unsafe {
            GetProcessImageFileNameW(GetCurrentProcess(), path.as_mut_ptr(), path.len() as u32)
        };
        if n == 0 {
            return Err(std::io::Error::last_os_error())
                .context("Cannot identify the bridge executable");
        }
        let path = String::from_utf16(&path[..n as usize])?;
        let mut allowed = self.list(2048)?;
        if !contains(&allowed, &path) {
            allowed.push(path);
            self.set_list(2049, &allowed)?;
        }
        Ok(())
    }
}
pub(super) fn contains(list: &[String], item: &str) -> bool {
    list.iter().any(|s| s.eq_ignore_ascii_case(item))
}
