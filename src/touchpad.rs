//! PC Precision Touchpad contacts, independent of XInput packet changes.
mod cursor;
mod raw;
pub mod validation;

use crate::protocol::PadState;
use anyhow::{Context, Result};
use serde::Serialize;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Contact {
    pub x: u16,
    pub y: u16,
    pub active: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct State {
    pub contacts: [Contact; 2],
    pub clicked: bool,
}
impl State {
    pub fn apply(self, pad: &mut PadState) {
        let [a, b] = self.contacts;
        pad.touch1_x = a.x;
        pad.touch1_y = a.y;
        pad.touch1_active = u8::from(a.active);
        pad.touch2_x = b.x;
        pad.touch2_y = b.y;
        pad.touch2_active = u8::from(b.active);
        pad.buttons = (pad.buttons & !0x20000) | if self.clicked { 0x20000 } else { 0 };
    }
    pub fn from_pad(p: &PadState) -> Self {
        Self {
            contacts: [
                Contact {
                    x: p.touch1_x,
                    y: p.touch1_y,
                    active: p.touch1_active != 0,
                },
                Contact {
                    x: p.touch2_x,
                    y: p.touch2_y,
                    active: p.touch2_active != 0,
                },
            ],
            clicked: p.buttons & 0x20000 != 0,
        }
    }
    pub fn from_virtual(report: &[u8]) -> Option<Self> {
        if report.len() != 64 || report[0] != 1 {
            return None;
        }
        // Sony uses the standard packet layout. The physical Raiju's PS5
        // packet has the third-party layout, with contacts one byte earlier.
        let contact = |offset: usize| Contact {
            active: report[offset] & 0x80 == 0,
            x: u16::from(report[offset + 1]) | (u16::from(report[offset + 2] & 15) << 8),
            y: u16::from(report[offset + 2] >> 4) | (u16::from(report[offset + 3]) << 4),
        };
        Some(Self {
            contacts: [contact(33), contact(37)],
            clicked: report[10] & 2 != 0,
        })
    }
}
#[derive(Default)]
struct Decoder {
    ids: [Option<u8>; 2],
    state: State,
}
impl Decoder {
    fn decode(&mut self, data: &[u8]) -> Option<State> {
        if data.len() != 20 || data[0] != 1 || data[18] > 3 {
            return None;
        }
        let contacts: Vec<_> = data[1..16]
            .as_chunks::<5>()
            .0
            .iter()
            .take(data[18] as usize)
            .filter(|c| c[0] & 3 == 3)
            .map(|c| {
                (
                    (c[0] >> 2) & 7,
                    Contact {
                        // The wired Raiju reports Sony's coordinate grid even
                        // though its PTP descriptor advertises 2628 x 1332.
                        // Four-corner capture verified 0..1919 / 0..1079.
                        x: u16::from_le_bytes([c[1], c[2]]).min(1919),
                        y: u16::from_le_bytes([c[3], c[4]]).min(1079),
                        active: true,
                    },
                )
            })
            .collect();
        for (slot, id) in self.ids.iter_mut().enumerate() {
            if id.is_some_and(|id| !contacts.iter().any(|(candidate, _)| *candidate == id)) {
                *id = None;
                self.state.contacts[slot].active = false;
            }
        }
        for (id, contact) in contacts {
            let slot = self
                .ids
                .iter()
                .position(|old| *old == Some(id))
                .or_else(|| self.ids.iter().position(Option::is_none));
            if let Some(slot) = slot {
                self.ids[slot] = Some(id);
                self.state.contacts[slot] = contact;
            }
        }
        self.state.clicked = data[19] & 1 != 0;
        Some(self.state)
    }
}
#[derive(Default)]
struct Shared {
    forwarding: bool,
    pending: VecDeque<State>,
    error: Option<String>,
}
pub struct Touchpad {
    reader: raw::Reader,
    cursor: cursor::Guard,
    shared: Arc<Mutex<Shared>>,
    current: State,
}
impl Touchpad {
    pub fn begin_forwarding(&mut self) {
        self.shared.lock().unwrap().forwarding = true;
    }
    pub fn open(pid: u16) -> Result<Self> {
        let path = raw::only_raiju(pid)?;
        let shared = Arc::new(Mutex::new(Shared::default()));
        let reader = raw::Reader::start(path, shared.clone())?;
        let cursor = cursor::Guard::start(pid)?;
        Ok(Self {
            reader,
            cursor,
            shared,
            current: State::default(),
        })
    }
    pub fn apply(&mut self, pad: &mut PadState) -> Result<bool> {
        let mut shared = self.shared.lock().unwrap();
        if let Some(error) = &shared.error {
            return Err(anyhow::anyhow!(error.clone()));
        }
        let next = shared.pending.pop_front().unwrap_or(self.current);
        let changed = next != self.current;
        self.current = next;
        next.apply(pad);
        Ok(changed)
    }
    pub fn check(&mut self) -> Result<()> {
        self.cursor
            .check()
            .context("PC touchpad cursor guard stopped")
    }
}
impl Drop for Touchpad {
    fn drop(&mut self) {
        self.reader.stop();
    }
}

/// Run before either executable initializes its normal application state.
pub fn watchdog_entry() -> bool {
    cursor::watchdog_entry()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn report(contacts: &[(u8, u16, u16)], click: bool) -> [u8; 20] {
        let mut b = [0; 20];
        b[0] = 1;
        b[18] = contacts.len() as u8;
        b[19] = u8::from(click);
        for (i, (id, x, y)) in contacts.iter().enumerate() {
            let off = 1 + 5 * i;
            b[off] = 3 | (id << 2);
            b[off + 1..off + 3].copy_from_slice(&x.to_le_bytes());
            b[off + 3..off + 5].copy_from_slice(&y.to_le_bytes());
        }
        b
    }
    #[test]
    fn endpoints_click_and_release() {
        let mut d = Decoder::default();
        let s = d
            .decode(&report(&[(0, 0, 0), (1, 2628, 1332)], true))
            .unwrap();
        assert_eq!(
            s.contacts,
            [
                Contact {
                    x: 0,
                    y: 0,
                    active: true
                },
                Contact {
                    x: 1919,
                    y: 1079,
                    active: true
                }
            ]
        );
        let mut p = PadState {
            buttons: 0x20,
            ..Default::default()
        };
        s.apply(&mut p);
        assert_eq!(p.buttons, 0x20020);
        let released = d.decode(&report(&[], false)).unwrap();
        released.apply(&mut p);
        assert_eq!(p.buttons, 0x20);
        assert_eq!([p.touch1_active, p.touch2_active], [0, 0]);
    }
    #[test]
    fn contacts_keep_slots_when_report_order_changes() {
        let mut d = Decoder::default();
        d.decode(&report(&[(2, 100, 200), (4, 400, 500)], false));
        let s = d
            .decode(&report(
                &[(4, 401, 501), (2, 101, 201), (5, 600, 700)],
                false,
            ))
            .unwrap();
        assert_eq!(d.ids, [Some(2), Some(4)]);
        assert_eq!(s.contacts[0].x, 101);
        d.decode(&report(&[(4, 401, 501), (5, 600, 700)], false));
        assert_eq!(d.ids, [Some(5), Some(4)]);
    }
    #[test]
    fn malformed_reports_and_unconfident_contacts() {
        let mut d = Decoder::default();
        assert!(d.decode(&[1; 19]).is_none());
        let mut b = report(&[(0, u16::MAX, u16::MAX)], false);
        assert_eq!(d.decode(&b).unwrap().contacts[0].x, 1919);
        b[1] &= !1;
        assert!(!d.decode(&b).unwrap().contacts[0].active);
        b[18] = 4;
        assert!(d.decode(&b).is_none());
    }
    #[test]
    fn sony_output_uses_standard_touch_offsets() {
        let mut report = [0; 64];
        report[0] = 1;
        report[10] = 2;
        report[33..37].copy_from_slice(&[0, 0xe8, 0xc3, 0x2b]);
        report[37] = 0x80;
        let s = State::from_virtual(&report).unwrap();
        assert_eq!(
            s.contacts[0],
            Contact {
                x: 1000,
                y: 700,
                active: true
            }
        );
        assert!(!s.contacts[1].active);
        assert!(s.clicked);
    }
    #[test]
    fn firmware_coordinates_are_not_scaled_to_descriptor_extents() {
        let mut d = Decoder::default();
        let s = d.decode(&report(&[(0, 960, 540)], false)).unwrap();
        assert_eq!(
            s.contacts[0],
            Contact {
                x: 960,
                y: 540,
                active: true
            }
        );
    }
}
