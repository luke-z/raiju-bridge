//! A minimal USB DualSense compatibility profile for the second experiment.
//! Wire facts: nondebug/dualsense USB descriptor; SDL's PS5 alternate layout.
//! No input automation, rumble, firmware flashing, or game-process access.
use crate::{Create, MAX_FEATURES, MAX_REPORT};
use anyhow::{Result, ensure};

pub fn descriptor() -> Vec<u8> {
    let mut d = vec![
        0x05, 0x01, 0x09, 0x05, 0xa1, 0x01, 0x85, 0x01, 0x09, 0x30, 0x09, 0x31, 0x09, 0x32, 0x09,
        0x35, 0x09, 0x33, 0x09, 0x34, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x06, 0x81,
        0x02, 0x06, 0x00, 0xff, 0x09, 0x20, 0x95, 0x01, 0x81, 0x02, 0x05, 0x01, 0x09, 0x39, 0x15,
        0x00, 0x25, 0x07, 0x35, 0x00, 0x46, 0x3b, 0x01, 0x65, 0x14, 0x75, 0x04, 0x95, 0x01, 0x81,
        0x42, 0x65, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x0f, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01,
        0x95, 0x0f, 0x81, 0x02, 0x06, 0x00, 0xff, 0x09, 0x21, 0x95, 0x0d, 0x81, 0x02, 0x06, 0x00,
        0xff, 0x09, 0x22, 0x15, 0x00, 0x26, 0xff, 0x00, 0x75, 0x08, 0x95, 0x34, 0x81, 0x02, 0x85,
        0x02, 0x09, 0x23, 0x95, 0x2f, 0x91, 0x02,
    ];
    for (id, usage, count) in [
        (0x05, 0x33, 40),
        (0x08, 0x34, 47),
        (0x09, 0x24, 19),
        (0x0a, 0x25, 26),
        (0x20, 0x26, 63),
        (0x21, 0x27, 4),
        (0x22, 0x40, 63),
        (0x80, 0x28, 63),
        (0x81, 0x29, 63),
        (0x82, 0x2a, 9),
        (0x83, 0x2b, 63),
        (0x84, 0x2c, 63),
        (0x85, 0x2d, 2),
        (0xa0, 0x2e, 1),
        (0xe0, 0x2f, 63),
        (0xf0, 0x30, 63),
        (0xf1, 0x31, 63),
        (0xf2, 0x32, 15),
        (0xf4, 0x35, 63),
        (0xf5, 0x36, 3),
    ] {
        d.extend_from_slice(&[0x85, id, 0x09, usage, 0x95, count, 0xb1, 0x02]);
    }
    d.push(0xc0);
    d
}

pub fn configure(c: &mut Create) {
    let descriptor = descriptor();
    c.descriptor.fill(0);
    c.descriptor[..descriptor.len()].copy_from_slice(&descriptor);
    c.descriptor_length = descriptor.len() as u16;
    c.input_length = 64;
    c.output_length = 48;
    c.feature_length = 64;
    c.revision = 0x0100;
    c.features = [[0; MAX_REPORT]; MAX_FEATURES];
    c.feature_lengths = [0; MAX_FEATURES];
    for (slot, (id, length)) in [(5, 41), (9, 20), (0x20, 64), (0x81, 64)]
        .into_iter()
        .enumerate()
    {
        c.feature_lengths[slot] = length;
        c.features[slot][0] = id;
    }
    let calibration: [i16; 17] = [
        0, 0, 0, 8192, -8192, 8192, -8192, 8192, -8192, 500, 500, 8192, -8192, 8192, -8192, 8192,
        -8192,
    ];
    for (i, value) in calibration.into_iter().enumerate() {
        c.features[0][1 + i * 2..3 + i * 2].copy_from_slice(&value.to_le_bytes());
    }
    c.features[0][35] = 0x0b;
    // Locally administered prototype address; not the physical controller's MAC.
    c.features[1][1..7].copy_from_slice(&[1, 0, 0, 0x56, 0x48, 2]);
    c.features[1][7] = 8;
    c.features[1][8] = 0x25;
    c.features[1][10] = 0x1e;
    c.features[1][12..16].copy_from_slice(&[0xee, 0x74, 0xd0, 0xbc]);
    let f = &mut c.features[2];
    f[1..12].copy_from_slice(b"Oct 30 2026");
    f[12..20].copy_from_slice(b"00:00:00");
    f[20] = 3;
    f[21] = 1;
    f[22] = 0x44;
    f[24..28].copy_from_slice(&0x01000208u32.to_le_bytes());
    f[28] = 0x36;
    f[31] = 1;
    f[32] = 0xc1;
    f[33] = 0xc8;
    f[44..46].copy_from_slice(&0x0630u16.to_le_bytes());
    f[48] = 0x14;
    f[52] = 0x0b;
    f[54] = 1;
    f[56] = 6;
    c.features[3][1] = 1;
    c.features[3][3..21].copy_from_slice(b"RAIJUVHFPROTO00001");
}

pub fn encode(raw: &[u8], sequence: u8, timestamp: u32) -> Result<[u8; 64]> {
    ensure!(
        raw.len() == 64 && raw[0] == 1,
        "Expected a 64-byte Raiju report with ID 1"
    );
    let mut out = [0u8; 64];
    // Core axes/triggers/buttons already occupy the same offsets. Preserve them.
    out[..11].copy_from_slice(&raw[..11]);
    out[7] = sequence;
    out[8] = (out[8] & 0xf0) | (out[8] & 15).min(8);
    out[10] &= 7;
    // Motion forwarding is absent in the existing bridge too: neutral gravity.
    out[26..28].copy_from_slice(&(-8192i16).to_le_bytes());
    out[28..32].copy_from_slice(&timestamp.to_le_bytes());
    out[33..37].copy_from_slice(&raw[32..36]);
    out[37..41].copy_from_slice(&raw[36..40]);
    out[49] = 0x10;
    // SDL's alternate PS5 layout places battery at offset 30 including report ID.
    // Its unavailable value is level 0xC; retain unknown instead of inventing 100%.
    let (state, level) = (raw[30] >> 4, raw[30] & 15);
    out[53] = if state <= 2 && level != 0x0c {
        (state << 4) | level.min(10)
    } else {
        0xf0
    };
    out[54] = 0x08; // The virtual compatibility profile is a wired controller.
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_descriptor_lengths_and_feature_replies() {
        let d = descriptor();
        assert_eq!(d.len(), 273);
        let lengths = crate::report_lengths(&d).unwrap();
        assert_eq!(lengths[&(0, 1)], 64);
        assert_eq!(lengths[&(1, 2)], 48);
        assert_eq!(lengths[&(2, 5)], 41);
        assert_eq!(lengths[&(2, 9)], 20);
        assert_eq!(lengths[&(2, 0x20)], 64);
        let mut c: Box<Create> = Box::new(unsafe { std::mem::zeroed() });
        configure(&mut c);
        for i in 0..4 {
            assert_eq!(
                lengths[&(2, c.features[i][0])],
                c.feature_lengths[i] as usize
            );
        }
        assert_eq!(
            i16::from_le_bytes(c.features[0][7..9].try_into().unwrap()),
            8192
        );
        assert_eq!(
            i16::from_le_bytes(c.features[0][9..11].try_into().unwrap()),
            -8192
        );
    }
    #[test]
    fn keeps_controls_and_moves_touch_and_battery() {
        let mut raw = [0u8; 64];
        raw[0] = 1;
        raw[1..7].copy_from_slice(&[0, 127, 128, 255, 50, 255]);
        raw[8..11].copy_from_slice(&[0x28, 0x91, 0x02]);
        raw[30] = 0x1b;
        raw[32..40].copy_from_slice(&[0, 0xe8, 0xc3, 0x2b, 0x81, 1, 2, 3]);
        let out = encode(&raw, 42, 123456).unwrap();
        assert_eq!(&out[1..7], &raw[1..7]);
        assert_eq!(&out[8..11], &raw[8..11]);
        assert_eq!(&out[33..41], &raw[32..40]);
        assert_eq!(out[53], 0x1a);
        assert_eq!(out[54], 8);
        assert_eq!(out[7], 42);
        assert_eq!(u32::from_le_bytes(out[28..32].try_into().unwrap()), 123456);
        raw[30] = 0x0c;
        assert_eq!(encode(&raw, 0, 0).unwrap()[53], 0xf0);
        assert!(encode(&raw[..40], 0, 0).is_err());
    }
}
