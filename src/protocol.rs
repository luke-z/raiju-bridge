#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PadState {
    pub lx: i8,
    pub ly: i8,
    pub rx: i8,
    pub ry: i8,
    pub buttons: u32,
    pub dpad: u8,
    pub l2: u8,
    pub r2: u8,
    pub touch1_x: u16,
    pub touch1_y: u16,
    pub touch1_active: u8,
    pub touch2_x: u16,
    pub touch2_y: u16,
    pub touch2_active: u8,
    pub gyro_x: i16,
    pub gyro_y: i16,
    pub gyro_z: i16,
    pub accel_x: i16,
    pub accel_y: i16,
    pub accel_z: i16,
}

impl Default for PadState {
    fn default() -> Self {
        Self {
            lx: 0,
            ly: 0,
            rx: 0,
            ry: 0,
            buttons: 0,
            dpad: 0,
            l2: 0,
            r2: 0,
            touch1_x: 0,
            touch1_y: 0,
            touch1_active: 0,
            touch2_x: 0,
            touch2_y: 0,
            touch2_active: 0,
            gyro_x: 0,
            gyro_y: 0,
            gyro_z: 0,
            accel_x: 0,
            accel_y: 0,
            accel_z: -8192,
        }
    }
}

pub fn hat_to_bits(hat: u8) -> u8 {
    match hat & 15 {
        0 => 1,
        1 => 9,
        2 => 8,
        3 => 10,
        4 => 2,
        5 => 6,
        6 => 4,
        7 => 5,
        _ => 0,
    }
}

pub fn parse_raiju(data: &[u8]) -> Option<PadState> {
    if data.len() < 10 || data[0] != 1 {
        return None;
    }
    let simple = matches!(data.len(), 10 | 78);
    if !simple && data.len() < 48 {
        return None;
    }
    let p = &data[1..];
    let b = if simple { 4 } else { 7 };
    let mut s = PadState {
        lx: (p[0] as i16 - 128) as i8,
        ly: (p[1] as i16 - 128) as i8,
        rx: (p[2] as i16 - 128) as i8,
        ry: (p[3] as i16 - 128) as i8,
        buttons: (p[b] & 0xf0) as u32 | ((p[b + 1] as u32) << 8) | (((p[b + 2] & 7) as u32) << 16),
        dpad: hat_to_bits(p[b]),
        l2: p[if simple { 7 } else { 4 }],
        r2: p[if simple { 8 } else { 5 }],
        ..PadState::default()
    };
    if s.l2 == 0 && s.buttons & 0x400 != 0 {
        s.l2 = 255;
    }
    if s.r2 == 0 && s.buttons & 0x800 != 0 {
        s.r2 = 255;
    }
    if !simple {
        s.touch1_active = u8::from(p[31] & 0x80 == 0);
        s.touch1_x = p[32] as u16 | (((p[33] & 15) as u16) << 8);
        s.touch1_y = (p[33] >> 4) as u16 | ((p[34] as u16) << 4);
        s.touch2_active = u8::from(p[35] & 0x80 == 0);
        s.touch2_x = p[36] as u16 | (((p[37] & 15) as u16) << 8);
        s.touch2_y = (p[37] >> 4) as u16 | ((p[38] as u16) << 4);
    }
    Some(s)
}

pub fn signature(s: &PadState) -> u32 {
    s.buttons | ((s.dpad as u32) << 24)
}

pub fn virtual_signature(report: &[u8]) -> Option<u32> {
    if report.len() != 64 || report[0] != 1 {
        return None;
    }
    Some(
        (report[8] & 0xf0) as u32
            | ((report[9] as u32) << 8)
            | (((report[10] & 7) as u32) << 16)
            | ((hat_to_bits(report[8]) as u32) << 24),
    )
}

pub fn button_names(mask: u32) -> String {
    const NAMES: &[(u32, &str)] = &[
        (0x10, "Square"),
        (0x20, "Cross"),
        (0x40, "Circle"),
        (0x80, "Triangle"),
        (0x100, "L1"),
        (0x200, "R1"),
        (0x400, "L2"),
        (0x800, "R2"),
        (0x1000, "Create"),
        (0x2000, "Options"),
        (0x4000, "L3"),
        (0x8000, "R3"),
        (0x10000, "PS"),
        (0x20000, "Touchpad"),
        (0x40000, "Mute"),
        (0x1000000, "D-pad up"),
        (0x2000000, "D-pad down"),
        (0x4000000, "D-pad left"),
        (0x8000000, "D-pad right"),
    ];
    NAMES
        .iter()
        .filter(|(bit, _)| mask & bit != 0)
        .map(|(_, name)| *name)
        .collect::<Vec<_>>()
        .join(" + ")
}

#[cfg(test)]
mod tests {
    use super::*;
    fn neutral() -> [u8; 64] {
        let mut p = [0; 64];
        p[0] = 1;
        p[1..5].fill(128);
        p[8] = 8;
        p[32] = 128;
        p[36] = 128;
        p
    }
    #[test]
    fn native_abi() {
        assert_eq!(std::mem::size_of::<PadState>(), 36);
        assert_eq!(std::mem::offset_of!(PadState, buttons), 4);
        assert_eq!(std::mem::offset_of!(PadState, accel_z), 34);
    }
    #[test]
    fn neutral_and_range() {
        let mut p = neutral();
        assert_eq!(parse_raiju(&p), Some(PadState::default()));
        p[1] = 0;
        p[2] = 255;
        let s = parse_raiju(&p).unwrap();
        assert_eq!((s.lx, s.ly), (-128, 127));
    }
    #[test]
    fn controls_and_touch() {
        let mut p = neutral();
        p[8] = 0x22;
        p[9] = 12;
        p[10] = 2;
        p[32] = 0;
        p[33] = 232;
        p[34] = 195;
        p[35] = 43;
        let s = parse_raiju(&p).unwrap();
        assert_eq!(s.buttons, 0x20c20);
        assert_eq!(s.dpad, 8);
        assert_eq!((s.l2, s.r2), (255, 255));
        assert_eq!((s.touch1_x, s.touch1_y, s.touch1_active), (1000, 700, 1));
        assert_eq!(virtual_signature(&p), Some(signature(&s)));
    }
    #[test]
    fn simple_and_malformed() {
        let s = parse_raiju(&[1, 128, 128, 128, 128, 0x28, 0x20, 2, 50, 100]).unwrap();
        assert_eq!((s.buttons, s.l2, s.r2), (0x22020, 50, 100));
        for len in 0..10 {
            assert!(parse_raiju(&vec![0; len]).is_none());
        }
        assert!(parse_raiju(&[0; 64]).is_none());
        assert!(parse_raiju(&[1; 20]).is_none());
    }
}
