//! Session-owned hiding of the physical PC gamepad, using the installed HidHide driver.
mod api;
mod devices;
mod session;
use anyhow::{Context, Result, bail};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::windows::process::CommandExt,
    process::{Child, Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

pub(crate) fn prepare() -> Result<u16> {
    // Recover before querying XInput: an interrupted older session may still hide it.
    session::recover()?;
    let found = devices::find()?.ok_or(crate::native::SourceUnavailable)?;
    let driver = api::Driver::open()?;
    driver.allow_current()?;
    Ok(found.pid)
}

pub(crate) struct Guard {
    child: Child,
    closing: bool,
}
impl Guard {
    pub fn start(pid: u16) -> Result<Self> {
        let mut child = Command::new(std::env::current_exe()?)
            .args(["--hidhide-watchdog", &pid.to_string()])
            .creation_flags(0x08000000)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdout = child
            .stdout
            .take()
            .context("HidHide watchdog has no response pipe")?;
        let mut guard = Self {
            child,
            closing: false,
        };
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            let mut line = String::new();
            let _ = tx.send(BufReader::new(stdout).read_line(&mut line).map(|_| line));
        });
        let reply = rx
            .recv_timeout(Duration::from_secs(5))
            .context("HidHide watchdog did not start")??;
        if reply.trim() != "READY" {
            bail!("HidHide: {}", reply.trim());
        }
        guard.check()?;
        Ok(guard)
    }
    pub fn check(&mut self) -> Result<()> {
        if self.child.try_wait()?.is_some() {
            session::recover()?;
            bail!(
                "HidHide cleanup helper stopped. The physical controller was restored; restart the bridge."
            );
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        self.closing = true;
        self.child.stdin.take();
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if self.child.try_wait()?.is_some() {
                return session::recover();
            }
            thread::sleep(Duration::from_millis(20));
        }
        // Keep the helper alive so closing a conflicting config client can unblock cleanup.
        bail!(
            "HidHide is busy restoring the physical gamepad. Close its configuration window; cleanup will continue automatically."
        )
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        if !self.closing
            && let Err(e) = self.finish()
        {
            eprintln!("{e:#}");
        }
    }
}

fn watchdog(pid: u16) -> Result<()> {
    let mut session = session::Session::start(pid)?;
    println!("READY");
    std::io::stdout().flush()?;
    // EOF occurs on normal Stop and on parent-process termination.
    let _ = std::io::stdin().read(&mut [0]);
    while session.restore().is_err() {
        thread::sleep(Duration::from_millis(200));
    }
    Ok(())
}
pub fn watchdog_entry() -> bool {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).map(String::as_str) != Some("--hidhide-watchdog") {
        return false;
    }
    let result = args
        .get(2)
        .context("Missing controller ID")
        .and_then(|s| s.parse::<u16>().map_err(Into::into))
        .and_then(|pid| {
            if !matches!(pid, 0x1025 | 0x1027) {
                bail!("Unsupported controller");
            }
            watchdog(pid)
        });
    if let Err(e) = result {
        println!("{e:#}");
        std::process::exit(1);
    }
    true
}
