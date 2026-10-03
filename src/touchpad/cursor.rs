//! Windows' cursor policy is global, so require a sole Raiju touchpad and mouse.
//! The helper owns the temporary setting and restores it on parent pipe closure.
use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};
use windows_sys::Win32::UI::WindowsAndMessaging::SystemParametersInfoW;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Parameters {
    version: u32,
    contacts: u32,
    legacy: u32,
    status: u32,
    settings: u32,
    other: [u32; 6],
}
fn parameters() -> Result<Parameters> {
    let mut p = Parameters {
        version: 1,
        ..Default::default()
    };
    if unsafe {
        SystemParametersInfoW(
            0xae,
            size_of::<Parameters>() as u32,
            (&mut p as *mut Parameters).cast(),
            0,
        )
    } == 0
    {
        bail!(
            "PC touchpad cursor control requires Windows 11 24H2 or newer: {}",
            std::io::Error::last_os_error()
        );
    }
    Ok(p)
}
fn set_allow_mouse(allow: bool) -> Result<()> {
    let mut p = parameters()?;
    p.settings = (p.settings & !1) | u32::from(allow);
    // No SPIF_UPDATEINIFILE: do not persist this session's policy across login.
    if unsafe {
        SystemParametersInfoW(
            0xaf,
            size_of::<Parameters>() as u32,
            (&mut p as *mut Parameters).cast(),
            0,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
struct Restore(bool);
impl Drop for Restore {
    fn drop(&mut self) {
        let _ = set_allow_mouse(self.0);
    }
}
fn check_environment(pid: u16) -> Result<Parameters> {
    super::raw::only_raiju(pid)?;
    let p = parameters()?;
    if p.status & 4 == 0 {
        bail!("Connect a regular mouse to suppress PC touchpad cursor input, or use PS5 mode.");
    }
    Ok(p)
}
pub(super) struct Guard {
    child: Child,
    original: bool,
}
impl Guard {
    pub fn start(pid: u16) -> Result<Self> {
        let original = check_environment(pid)?.settings & 1 != 0;
        let mut child = Command::new(std::env::current_exe()?)
            .args(["--touchpad-watchdog", &pid.to_string()])
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .context("Touchpad watchdog has no response pipe")?;
        let mut guard = Self { child, original };
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut line = String::new();
            let result = BufReader::new(stdout).read_line(&mut line).map(|_| line);
            let _ = tx.send(result);
        });
        let reply = rx
            .recv_timeout(Duration::from_secs(5))
            .context("Touchpad watchdog did not start")??;
        if reply.trim() != "READY" {
            bail!("Touchpad cursor control: {}", reply.trim());
        }
        guard.check()?;
        Ok(guard)
    }
    pub fn check(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_some() {
            bail!(
                "Touchpad environment changed. Keep a regular mouse connected and other Precision Touchpads disconnected, then restart."
            );
        }
        Ok(())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        // EOF also occurs if the parent is terminated. The helper restores then exits.
        self.child.stdin.take();
        for _ in 0..100 {
            if self.child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        // Covers an independently terminated helper. Only the owned field changes.
        let _ = set_allow_mouse(self.original);
    }
}
fn watchdog(pid: u16) -> Result<()> {
    let p = check_environment(pid)?;
    let restore = Restore(p.settings & 1 != 0);
    set_allow_mouse(false)?;
    let stopped = Arc::new(AtomicBool::new(false));
    let flag = stopped.clone();
    thread::spawn(move || {
        let mut buffer = [0; 1];
        let _ = std::io::stdin().read(&mut buffer);
        flag.store(true, Ordering::Release);
    });
    println!("READY");
    std::io::stdout().flush()?;
    let mut ticks = 0;
    while !stopped.load(Ordering::Acquire) {
        thread::sleep(Duration::from_millis(100));
        ticks += 1;
        if ticks % 10 == 0 {
            let p = check_environment(pid)?;
            if p.status & 16 != 0 {
                bail!("Windows touchpad cursor policy changed");
            }
        }
    }
    drop(restore);
    Ok(())
}
pub(super) fn watchdog_entry() -> bool {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("--touchpad-watchdog") {
        return false;
    }
    let result = args
        .get(2)
        .context("Missing touchpad ID")
        .and_then(|s| s.parse::<u16>().map_err(Into::into))
        .and_then(|pid| {
            if !matches!(pid, 0x1025 | 0x1027) {
                bail!("Unsupported touchpad");
            }
            watchdog(pid)
        });
    if let Err(e) = result {
        println!("{e:#}");
        std::process::exit(1);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn abi_matches_touchpad_parameters_v1() {
        assert_eq!(size_of::<Parameters>(), 44);
        assert_eq!(std::mem::offset_of!(Parameters, settings), 16);
    }
}
