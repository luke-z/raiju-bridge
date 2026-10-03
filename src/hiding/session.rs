use super::{
    api::{Driver, contains},
    devices,
};
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::windows::fs::OpenOptionsExt,
    path::PathBuf,
};

#[derive(Serialize, Deserialize)]
struct Record {
    added: Vec<String>,
    activated: bool,
}
impl Record {
    fn plan(existing: &[String], active: bool, wanted: &[String]) -> Result<Self> {
        if !active && existing.iter().any(|id| !contains(wanted, id)) {
            bail!(
                "HidHide has disabled rules for other devices. Enable hiding in its configuration first; the bridge will not activate those rules automatically."
            );
        }
        Ok(Self {
            added: wanted
                .iter()
                .filter(|id| !contains(existing, id))
                .cloned()
                .collect(),
            activated: !active,
        })
    }
    fn restore(&self) -> Result<()> {
        if self.added.len() > 2 || self.added.iter().any(|id| !devices::valid(id)) {
            bail!("Invalid HidHide recovery record; no device configuration was changed");
        }
        let driver = Driver::open()?;
        let mut blocked = driver.list(2050)?;
        let before = blocked.len();
        blocked.retain(|id| !contains(&self.added, id));
        if blocked.len() != before {
            driver.set_list(2051, &blocked)?;
        }
        // Never disable rules another application added during this session.
        if self.activated && blocked.is_empty() {
            driver.set_flag(2053, false)?;
        }
        Ok(())
    }
}
fn path() -> PathBuf {
    crate::desktop::data_root().join("hidhide-session.json")
}
fn remove_record() -> Result<()> {
    match fs::remove_file(path()) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub(super) fn recover() -> Result<()> {
    let mut file = match OpenOptions::new().read(true).write(true).share_mode(0).open(path()) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e).context("The previous HidHide cleanup is still running. Close HidHide's configuration window and retry"),
    };
    if file.metadata()?.len() > 4096 {
        bail!("Oversized HidHide recovery record");
    }
    let mut data = String::new();
    file.read_to_string(&mut data)?;
    let record: Record =
        serde_json::from_str(&data).context("Cannot read HidHide recovery record")?;
    record.restore()?;
    drop(file);
    remove_record()?;
    Ok(())
}

pub(super) struct Session {
    record: Record,
    // Exclusive file lock distinguishes a running cleanup helper from stale data.
    file: Option<File>,
}
impl Session {
    pub fn start(pid: u16) -> Result<Self> {
        let selection = devices::find()?.context("Raiju PC gamepad disconnected")?;
        if selection.pid != pid {
            bail!("Raiju changed while connecting; retry Start");
        }
        let driver = Driver::open()?;
        driver.allow_current()?;
        let mut blocked = driver.list(2050)?;
        let record = Record::plan(&blocked, driver.flag(2052)?, &selection.ids)?;
        fs::create_dir_all(crate::desktop::data_root())?;
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(0)
            .open(path())
            .context("Another HidHide session or recovery record exists")?;
        let saved = (|| -> Result<()> {
            file.write_all(&serde_json::to_vec(&record)?)?;
            file.sync_all()?;
            Ok(())
        })();
        if let Err(e) = saved {
            drop(file);
            let _ = fs::remove_file(path());
            return Err(e);
        }
        let mut session = Self {
            record,
            file: Some(file),
        };
        blocked.extend(session.record.added.iter().cloned());
        let applied = (|| -> Result<()> {
            driver.set_list(2051, &blocked)?;
            driver.set_flag(2053, true)?;
            Ok(())
        })();
        drop(driver); // The control device permits only one open client.
        if let Err(e) = applied {
            let _ = session.restore();
            return Err(e);
        }
        Ok(session)
    }
    pub fn restore(&mut self) -> Result<()> {
        self.record.restore()?;
        self.file.take();
        remove_record()?;
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if self.file.is_some() {
            let _ = self.restore();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_owns_only_rules_added_by_this_session() {
        let wanted: Vec<String> = vec![
            r"USB\VID_1532&PID_1025&MI_00\ONE".into(),
            r"HID\VID_1532&PID_1025&IG_02\ONE".into(),
        ];
        let existing = vec![wanted[0].clone(), "OTHER DEVICE".into()];
        let r = Record::plan(&existing, true, &wanted).unwrap();
        assert_eq!(r.added, wanted[1..]);
        assert!(!r.activated);
        assert!(Record::plan(&existing, false, &wanted).is_err());
        let r = Record::plan(&[], false, &wanted).unwrap();
        assert_eq!(r.added, wanted);
        assert!(r.activated);
    }
}
