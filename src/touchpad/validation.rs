use super::{Contact, State};
use crate::{native, protocol::PadState};
use anyhow::{Result, bail};
use std::{
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

/// Independent HID readback of two contacts, coordinates, release and click.
pub fn sweep(directory: &Path) -> Result<()> {
    let _exclusive = native::ExclusiveSession::acquire()?;
    let previous = native::sony_paths()?;
    let pad = native::VirtualPad::attach(1, &AtomicBool::new(false))?;
    let input = native::wait_for_virtual(&previous, &AtomicBool::new(false))?;
    let mut rows = Vec::new();
    for index in 0..128u16 {
        let expected = State {
            contacts: [
                Contact {
                    x: ((u32::from(index) * 1919) / 127) as u16,
                    y: ((u32::from(index) * 1079) / 127) as u16,
                    active: index & 1 != 0,
                },
                Contact {
                    x: 1919 - ((u32::from(index) * 1919) / 127) as u16,
                    y: 1079 - ((u32::from(index) * 1079) / 127) as u16,
                    active: index & 2 != 0,
                },
            ],
            clicked: index & 4 != 0,
        };
        let mut state = PadState {
            gyro_x: (index + 1) as i16,
            ..Default::default()
        };
        expected.apply(&mut state);
        pad.set(state)?;
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut report = [0; 128];
        let actual = loop {
            if Instant::now() > deadline {
                bail!("Touch vector {index} timed out");
            }
            let n = input.read_timeout(&mut report, 50)?;
            if n == 64 && u16::from_le_bytes([report[16], report[17]]) == index + 1 {
                break State::from_virtual(&report[..n]).unwrap();
            }
        };
        rows.push(serde_json::json!({"index":index,"expected":expected,"actual":actual,"equal":expected==actual}));
    }
    let matched = rows.iter().filter(|r| r["equal"] == true).count();
    std::fs::create_dir_all(directory)?;
    std::fs::write(
        directory.join("touch-sweep.json"),
        serde_json::to_vec_pretty(
            &serde_json::json!({"tested":rows.len(),"matched":matched,"vectors":rows}),
        )?,
    )?;
    if matched != rows.len() {
        bail!("Touch sweep: {} mismatches", rows.len() - matched);
    }
    println!("Touch sweep: {matched} / {} exact", rows.len());
    Ok(())
}
