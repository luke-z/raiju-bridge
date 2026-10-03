use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    ptr,
};
use windows_sys::Win32::{
    Foundation::*,
    System::{Registry::*, Threading::*},
};

pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}
pub fn data_root() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_exe()
                .unwrap()
                .parent()
                .unwrap()
                .to_path_buf()
        })
        .join("RaijuBridge")
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub auto_connect: bool,
    pub diagnostics: bool,
    pub precise_pc: bool,
}
impl Settings {
    pub fn load() -> Result<Self> {
        let file = data_root().join("settings.json");
        match fs::read(file) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .context("Settings could not be read. The original file was preserved."),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(data_root())?;
        let file = data_root().join("settings.json");
        let temp = data_root().join("settings.json.tmp");
        fs::write(&temp, serde_json::to_vec_pretty(self)?)?;
        fs::rename(temp, file)?;
        Ok(())
    }
}
const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";
const RUN_NAME: &str = "RaijuBridge";
pub fn startup_command(exe: &Path) -> Result<String> {
    let path = exe
        .to_str()
        .context("The executable path is not valid Unicode")?;
    if path.contains('"') {
        bail!("Invalid executable path");
    }
    Ok(format!("\"{path}\" --background"))
}
pub fn startup_value() -> Result<Option<String>> {
    let mut bytes = 0u32;
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(RUN_NAME).as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            ptr::null_mut(),
            &mut bytes,
        )
    };
    if code == ERROR_FILE_NOT_FOUND {
        return Ok(None);
    }
    if code != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(code as i32).into());
    }
    let mut value = vec![0u16; (bytes as usize).div_ceil(2)];
    let code = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            wide(RUN_NAME).as_ptr(),
            RRF_RT_REG_SZ,
            ptr::null_mut(),
            value.as_mut_ptr().cast(),
            &mut bytes,
        )
    };
    if code != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(code as i32).into());
    }
    let end = value.iter().position(|&x| x == 0).unwrap_or(value.len());
    Ok(Some(String::from_utf16_lossy(&value[..end])))
}
pub fn set_startup(enabled: bool) -> Result<()> {
    let value = if enabled {
        Some(wide(&startup_command(&std::env::current_exe()?)?))
    } else {
        None
    };
    let mut key = ptr::null_mut();
    let code = unsafe {
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            wide(RUN_KEY).as_ptr(),
            0,
            ptr::null(),
            0,
            KEY_SET_VALUE,
            ptr::null(),
            &mut key,
            ptr::null_mut(),
        )
    };
    if code != ERROR_SUCCESS {
        return Err(std::io::Error::from_raw_os_error(code as i32).into());
    }
    let code = unsafe {
        let code = if let Some(value) = &value {
            RegSetValueExW(
                key,
                wide(RUN_NAME).as_ptr(),
                0,
                REG_SZ,
                value.as_ptr().cast(),
                (value.len() * 2) as u32,
            )
        } else {
            RegDeleteValueW(key, wide(RUN_NAME).as_ptr())
        };
        RegCloseKey(key);
        code
    };
    if code != ERROR_SUCCESS && !(code == ERROR_FILE_NOT_FOUND && !enabled) {
        return Err(std::io::Error::from_raw_os_error(code as i32).into());
    }
    Ok(())
}

pub struct AppInstance {
    mutex: HANDLE,
    show: HANDLE,
}
impl AppInstance {
    pub fn acquire() -> Result<Option<Self>> {
        unsafe {
            let show = CreateEventW(
                ptr::null(),
                0,
                0,
                wide("Local\\RaijuBridge.Show.v03").as_ptr(),
            );
            if show.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            let mutex = CreateMutexW(ptr::null(), 1, wide("Local\\RaijuBridge.App.v03").as_ptr());
            let existing = GetLastError() == ERROR_ALREADY_EXISTS;
            if mutex.is_null() {
                CloseHandle(show);
                return Err(std::io::Error::last_os_error().into());
            }
            if existing {
                SetEvent(show);
                CloseHandle(show);
                CloseHandle(mutex);
                return Ok(None);
            }
            Ok(Some(Self { mutex, show }))
        }
    }
    pub fn take_show_request(&self) -> bool {
        unsafe { WaitForSingleObject(self.show, 0) == WAIT_OBJECT_0 }
    }
}
impl Drop for AppInstance {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.mutex);
            CloseHandle(self.mutex);
            CloseHandle(self.show);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoted_startup_path_and_safe_defaults() {
        assert_eq!(
            startup_command(Path::new("C:\\Some Folder\\bridge.exe")).unwrap(),
            "\"C:\\Some Folder\\bridge.exe\" --background"
        );
        let settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(!settings.auto_connect && !settings.diagnostics && !settings.precise_pc);
        let saved: Settings = serde_json::from_str(r#"{"precise_pc":true}"#).unwrap();
        assert!(saved.precise_pc);
        assert!(startup_command(Path::new("C:\\bad\"path.exe")).is_err());
    }
}
