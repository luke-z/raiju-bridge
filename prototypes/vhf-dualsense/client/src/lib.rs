use anyhow::{Context, Result, bail, ensure};
use hidapi::{HidApi, HidDevice};
use serde_json::json;
mod dualsense;
pub mod runtime;
use std::{
    collections::BTreeMap,
    mem::size_of,
    ptr,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use windows_sys::{
    Win32::{
        Devices::DeviceAndDriverInstallation::*,
        Foundation::*,
        Storage::FileSystem::*,
        System::{Console::SetConsoleCtrlHandler, IO::DeviceIoControl},
    },
    core::GUID,
};

const INTERFACE: GUID = GUID::from_u128(0x3a04ce2a_b6ba_4f17_8342_d894e6614471);
const CREATE: u32 = (0x22 << 16) | (2 << 14) | (0x800 << 2);
const INPUT: u32 = (0x22 << 16) | (2 << 14) | (0x801 << 2);
const STATUS: u32 = (0x22 << 16) | (1 << 14) | (0x802 << 2);
const DESTROY: u32 = (0x22 << 16) | (2 << 14) | (0x803 << 2);
const MAX_DESCRIPTOR: usize = 4096;
const MAX_REPORT: usize = 128;
const MAX_FEATURES: usize = 16;
static STOP: AtomicBool = AtomicBool::new(false);

#[repr(C)]
struct Create {
    version: u32,
    vid: u16,
    pid: u16,
    revision: u16,
    descriptor_length: u16,
    input_length: u16,
    output_length: u16,
    feature_length: u16,
    reserved: u16,
    descriptor: [u8; MAX_DESCRIPTOR],
    features: [[u8; MAX_REPORT]; MAX_FEATURES],
    feature_lengths: [u16; MAX_FEATURES],
}
#[repr(C)]
#[derive(Default, Debug)]
struct Status {
    version: u32,
    active: u32,
    submitted: u32,
    get_input: u32,
    get_feature: u32,
    set_feature: u32,
    output: u32,
    unsupported_feature: u32,
    last_feature_id: u32,
    last_submit_status: u32,
    watchdog_resets: u32,
    last_set_feature: u32,
}

struct Control(HANDLE);
impl Control {
    fn open() -> Result<Self> {
        let mut length = 0;
        let cr = unsafe {
            CM_Get_Device_Interface_List_SizeW(
                &mut length,
                &INTERFACE,
                ptr::null(),
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        ensure!(cr == 0, "Interface enumeration failed: CONFIGRET {cr}");
        ensure!(
            length > 1,
            "Raiju VHF prototype driver is not installed or not started"
        );
        let mut paths = vec![0u16; length as usize];
        let cr = unsafe {
            CM_Get_Device_Interface_ListW(
                &INTERFACE,
                ptr::null(),
                paths.as_mut_ptr(),
                length,
                CM_GET_DEVICE_INTERFACE_LIST_PRESENT,
            )
        };
        ensure!(
            cr == 0 && paths[0] != 0,
            "Prototype interface not available: CONFIGRET {cr}"
        );
        let handle = unsafe {
            CreateFileW(
                paths.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                ptr::null(),
                OPEN_EXISTING,
                0,
                ptr::null_mut(),
            )
        };
        ensure!(
            handle != INVALID_HANDLE_VALUE,
            "Open prototype: {}",
            std::io::Error::last_os_error()
        );
        Ok(Self(handle))
    }
    fn ioctl(&self, code: u32, input: &[u8], output: &mut [u8]) -> Result<usize> {
        let mut returned = 0;
        let ok = unsafe {
            DeviceIoControl(
                self.0,
                code,
                input.as_ptr().cast(),
                input.len() as u32,
                output.as_mut_ptr().cast(),
                output.len() as u32,
                &mut returned,
                ptr::null_mut(),
            )
        };
        ensure!(
            ok != 0,
            "IOCTL {code:#x}: {}",
            std::io::Error::last_os_error()
        );
        Ok(returned as usize)
    }
    fn status(&self) -> Result<Status> {
        let mut status = Status::default();
        let bytes = unsafe {
            std::slice::from_raw_parts_mut((&mut status as *mut Status).cast(), size_of::<Status>())
        };
        ensure!(
            self.ioctl(STATUS, &[], bytes)? == size_of::<Status>(),
            "Unexpected status size"
        );
        ensure!(status.version == 1, "Driver protocol mismatch");
        Ok(status)
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

// Returns per-type, per-ID report bytes, including the report ID. Reconstructed
// descriptors preserve bit layout, not necessarily the original byte encoding.
fn report_lengths(d: &[u8]) -> Result<BTreeMap<(u8, u8), usize>> {
    let mut sizes = BTreeMap::new();
    let (mut bits, mut count, mut id) = (0usize, 0usize, 0u8);
    let mut stack = Vec::new();
    let mut offset = 0;
    while offset < d.len() {
        let prefix = d[offset];
        offset += 1;
        ensure!(
            prefix != 0xfe,
            "Long HID item unsupported by prototype inspector"
        );
        let n = [0, 1, 2, 4][(prefix & 3) as usize];
        ensure!(offset + n <= d.len(), "Truncated HID descriptor");
        let mut value = 0usize;
        for i in 0..n {
            value |= (d[offset + i] as usize) << (8 * i);
        }
        offset += n;
        match prefix & 0xfc {
            0x74 => bits = value,
            0x94 => count = value,
            0x84 => {
                ensure!(value > 0 && value <= 255, "Invalid report ID");
                id = value as u8;
            }
            0xa4 => stack.push((bits, count, id)),
            0xb4 => {
                (bits, count, id) = stack.pop().context("Unbalanced HID pop")?;
            }
            0x80 | 0x90 | 0xb0 => {
                let kind = match prefix & 0xfc {
                    0x80 => 0,
                    0x90 => 1,
                    _ => 2,
                };
                let add = bits.checked_mul(count).context("Report length overflow")?;
                let current = sizes.entry((kind, id)).or_insert(0usize);
                *current = current.checked_add(add).context("Report length overflow")?;
                ensure!(*current <= MAX_REPORT * 8, "Report too large for prototype");
            }
            _ => (),
        }
    }
    ensure!(stack.is_empty(), "Unbalanced HID push");
    Ok(sizes
        .into_iter()
        .map(|(key, bits)| (key, bits.div_ceil(8) + 1))
        .collect())
}

fn source() -> Result<(HidDevice, u16, u16)> {
    let api = HidApi::new()?;
    let candidates: Vec<_> = api
        .device_list()
        .filter(|d| {
            d.vendor_id() == 0x1532
                && matches!(d.product_id(), 0x1024 | 0x1026)
                && d.usage_page() == 1
                && matches!(d.usage(), 4 | 5)
        })
        .collect();
    ensure!(
        candidates.len() == 1,
        "Connect exactly one Raiju in PS mode; found {}",
        candidates.len()
    );
    let d = candidates[0];
    Ok((d.open_device(&api)?, d.product_id(), d.release_number()))
}

fn prepare(identity: &str) -> Result<(HidDevice, Box<Create>, serde_json::Value)> {
    let (device, pid, revision) = source()?;
    let mut create: Box<Create> = Box::new(unsafe { std::mem::zeroed() });
    create.version = 1;
    (create.vid, create.pid, create.revision) = match identity {
        "sony" | "dualsense" => (0x054c, 0x0ce6, revision),
        "razer" => (0x1532, pid, revision),
        _ => bail!("Profile must be sony, razer or dualsense"),
    };
    let n = device.get_report_descriptor(&mut create.descriptor)?;
    ensure!(
        n > 0 && n <= MAX_DESCRIPTOR,
        "Invalid reconstructed descriptor length"
    );
    create.descriptor_length = n as u16;
    let mut lengths = report_lengths(&create.descriptor[..n])?;
    create.input_length = lengths
        .iter()
        .filter(|((kind, _), _)| *kind == 0)
        .map(|(_, length)| *length)
        .max()
        .unwrap_or(0) as u16;
    create.output_length = lengths
        .iter()
        .filter(|((kind, _), _)| *kind == 1)
        .map(|(_, length)| *length)
        .max()
        .unwrap_or(0) as u16;
    create.feature_length = lengths
        .iter()
        .filter(|((kind, _), _)| *kind == 2)
        .map(|(_, length)| *length)
        .max()
        .unwrap_or(0) as u16;
    ensure!(
        create.input_length == 64,
        "Identity-only test currently requires a 64-byte input report; got {}",
        create.input_length
    );
    ensure!(
        create.feature_length as usize <= MAX_REPORT,
        "Feature report too large"
    );
    let mut features = Vec::new();
    let feature_ids: Vec<_> = lengths
        .iter()
        .filter(|((kind, _), _)| *kind == 2)
        .map(|((_, id), length)| (*id, *length))
        .collect();
    ensure!(
        feature_ids.len() <= MAX_FEATURES,
        "Too many feature reports for prototype"
    );
    for (slot, (id, length)) in feature_ids.into_iter().enumerate() {
        let capacity = create.feature_length as usize;
        create.features[slot][0] = id;
        match device.get_feature_report(&mut create.features[slot][..capacity]) {
            Ok(read) => {
                create.feature_lengths[slot] = length.min(read) as u16;
                features
                    .push(json!({"id":id,"expected_bytes":length,"read_bytes":read,"cached":true}));
            }
            Err(e) => features.push(json!({"id":id,"cached":false,"error":e.to_string()})),
        }
    }
    if identity == "dualsense" {
        dualsense::configure(&mut create);
        lengths = report_lengths(&create.descriptor[..create.descriptor_length as usize])?;
        features = (0..4).map(|i|json!({"id":create.features[i][0],"bytes":create.feature_lengths[i],"source":"prototype compatibility fixture"})).collect();
    }
    let info = json!({
        "source_vid":0x1532,"source_pid":pid,"source_revision":revision,
        "target_vid":create.vid,"target_pid":create.pid,"target_revision":create.revision,
        "descriptor_bytes":create.descriptor_length,"descriptor_origin":if identity == "dualsense" {"USB DualSense compatibility descriptor"} else {"hidapi reconstructed from Windows preparsed data"},
        "input_bytes":create.input_length,"output_bytes":create.output_length,"feature_bytes":create.feature_length,
        "reports": lengths.iter().map(|((kind,id),length)| json!({"type":(["input","output","feature"][*kind as usize]),"id":id,"bytes":length})).collect::<Vec<_>>(),
        "feature_queries":features,"mode":if identity == "dualsense" {"DualSense descriptor, basic features and input translation; no VIIPER/USB-IP"} else {"identity-only; unmodified physical input; no VIIPER/USB-IP"}
    });
    Ok((device, create, info))
}

unsafe extern "system" fn console_handler(_: u32) -> i32 {
    STOP.store(true, Ordering::Relaxed);
    1
}

pub fn cli_main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("handshake") => {
            ensure!(
                Control::open()?.status()?.active == 1,
                "No active virtual session"
            );
            let api = HidApi::new()?;
            let candidates: Vec<_> = api
                .device_list()
                .filter(|d| {
                    d.vendor_id() == 0x054c
                        && d.product_id() == 0x0ce6
                        && d.usage_page() == 1
                        && d.usage() == 5
                })
                .collect();
            ensure!(candidates.len() == 1, "Expected exactly one Sony profile");
            let device = candidates[0].open_device(&api)?;
            let mut pairing = [0u8; 64];
            pairing[0] = 9;
            device.get_feature_report(&mut pairing)?;
            ensure!(
                pairing[1..7] == [1, 0, 0, 0x56, 0x48, 2],
                "Refusing feature write: this is not the owned prototype"
            );
            let mut evidence = Vec::new();
            for command in [1u8, 3, 4] {
                let mut request = [0u8; 64];
                request[0] = 0x80;
                request[1] = command;
                device.send_feature_report(&request)?;
                let mut reply = [0u8; 64];
                reply[0] = 0x81;
                device.get_feature_report(&mut reply)?;
                ensure!(reply[0] == 0x81, "Wrong response ID");
                match command {
                    1 => ensure!(
                        &reply[3..21] == b"RAIJUVHFPROTO00001",
                        "Serial fixture mismatch"
                    ),
                    3 => ensure!(reply[1] == 1 && reply[4] == 1, "Status fixture mismatch"),
                    4 => ensure!(
                        reply[4..8] == [0xd8, 0x0e, 0xb6, 0x08],
                        "Sensor fixture mismatch"
                    ),
                    _ => unreachable!(),
                }
                evidence.push(json!({"command":command,"accepted":true,"response_valid":true}));
            }
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"virtual_handshake":evidence,"status":format!("{:?}",Control::open()?.status()?)})
                )?
            );
        }
        Some("verify") => {
            let api = HidApi::new()?;
            let devices: Vec<_> = api
                .device_list()
                .filter(|d| {
                    d.vendor_id() == 0x054c
                        && d.product_id() == 0x0ce6
                        && d.usage_page() == 1
                        && matches!(d.usage(), 4 | 5)
                })
                .collect();
            ensure!(
                devices.len() == 1,
                "Expected one Sony-identity test controller, found {}",
                devices.len()
            );
            let info = devices[0];
            let device = info.open_device(&api)?;
            let mut descriptor = [0u8; MAX_DESCRIPTOR];
            let descriptor_bytes = device.get_report_descriptor(&mut descriptor)?;
            let lengths = report_lengths(&descriptor[..descriptor_bytes])?;
            ensure!(
                lengths.get(&(0, 1)) == Some(&64),
                "Virtual input layout differs"
            );
            ensure!(
                lengths.get(&(1, 2)) == Some(&48),
                "Virtual output layout differs"
            );
            let mut report = [0u8; MAX_REPORT];
            let bytes = device.read_timeout(&mut report, 1500)?;
            ensure!(
                bytes == 64 && report[0] == 1,
                "Virtual input is not readable: {bytes} bytes"
            );
            let feature_id = if lengths.contains_key(&(2, 5)) { 5 } else { 3 };
            let feature_max = lengths
                .iter()
                .filter(|((kind, _), _)| *kind == 2)
                .map(|(_, n)| *n)
                .max()
                .unwrap_or(0);
            let mut feature = vec![0u8; feature_max];
            ensure!(!feature.is_empty(), "No virtual feature reports advertised");
            feature[0] = feature_id;
            let feature_result = device
                .get_feature_report(&mut feature)
                .map(|n| json!({"bytes":n,"id":feature[0]}))
                .unwrap_or_else(|e| json!({"error":e.to_string()}));
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &json!({"vid":info.vendor_id(),"pid":info.product_id(),"revision":info.release_number(),"descriptor_bytes":descriptor_bytes,"input_read_bytes":bytes,"input_report_id":report[0],"raw_byte_30":report[30],"raw_byte_53":report[53],"feature_query":feature_result,"status":format!("{:?}",Control::open()?.status()?)})
                )?
            );
        }
        Some("probe") => {
            let (device, _, mut info) = prepare("razer")?;
            let mut raw = [0u8; MAX_REPORT];
            let n = device.read_timeout(&mut raw, 1000)?;
            info["sample_bytes"] = json!(n);
            info["sample_report_id"] = if n > 0 { json!(raw[0]) } else { json!(null) };
            info["control_driver"] = match Control::open().and_then(|c| c.status()) {
                Ok(s) => json!(format!("{s:?}")),
                Err(e) => json!(e.to_string()),
            };
            println!("{}", serde_json::to_string_pretty(&info)?);
        }
        Some("run") => {
            let identity = args.get(1).map(String::as_str).unwrap_or("sony");
            unsafe {
                SetConsoleCtrlHandler(Some(console_handler), 1);
            }
            forward(
                identity,
                || STOP.load(Ordering::Relaxed),
                || println!("Profile {identity} active. Ctrl+C removes the HID child."),
                |status| println!("{status:?}"),
            )?;
        }
        Some("status") => println!("{:?}", Control::open()?.status()?),
        _ => println!(
            "raiju-vhf-probe probe | verify | handshake | run [sony|razer|dualsense] | status"
        ),
    }
    Ok(())
}

// The tray and diagnostic CLI share the same descriptor, report translation and
// driver protocol. Check cancellation after reads and before input submission.
fn forward(
    identity: &str,
    stop: impl Fn() -> bool,
    started: impl FnOnce(),
    mut tick: impl FnMut(&Status),
) -> Result<()> {
    let (device, create, _) = prepare(identity)?;
    if stop() {
        return Ok(());
    }
    let control = Control::open()?;
    let bytes = unsafe {
        std::slice::from_raw_parts((&*create as *const Create).cast(), size_of::<Create>())
    };
    control.ioctl(CREATE, bytes, &mut [])?;
    let mut raw = [0u8; MAX_REPORT];
    let (origin, mut last_frame, mut last_tick) = (Instant::now(), Instant::now(), Instant::now());
    let mut sequence = 0u8;
    let mut started = Some(started);
    let result: Result<()> = (|| {
        while !stop() {
            let n = device.read_timeout(&mut raw, 50)?;
            if stop() {
                break;
            }
            if n > 0 {
                ensure!(
                    n == create.input_length as usize && raw[0] == 1,
                    "Unexpected physical report: {n} bytes, ID {}",
                    raw[0]
                );
                if identity == "dualsense" {
                    let report = dualsense::encode(
                        &raw[..n],
                        sequence,
                        origin.elapsed().as_micros().wrapping_mul(3) as u32,
                    )?;
                    sequence = sequence.wrapping_add(1);
                    control.ioctl(INPUT, &report, &mut [])?;
                } else {
                    control.ioctl(INPUT, &raw[..n], &mut [])?;
                }
                last_frame = Instant::now();
                if let Some(started) = started.take() {
                    started();
                }
            }
            ensure!(
                last_frame.elapsed() < Duration::from_millis(200),
                "Physical controller stopped sending reports"
            );
            if last_tick.elapsed() >= Duration::from_secs(2) {
                let status = control.status()?;
                ensure!(status.active == 1, "Virtual controller session stopped");
                tick(&status);
                last_tick = Instant::now();
            }
        }
        Ok(())
    })();
    let cleanup = control.ioctl(DESTROY, &[], &mut []);
    result?;
    cleanup?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_lengths_and_global_stack() {
        let d = [
            0x85, 1, 0x75, 8, 0x95, 63, 0x81, 2, 0xa4, 0x85, 2, 0x95, 47, 0x91, 2, 0xb4, 0x81, 2,
        ];
        let lengths = report_lengths(&d).unwrap();
        assert_eq!(lengths[&(0, 1)], 127);
        assert_eq!(lengths[&(1, 2)], 48);
        assert!(report_lengths(&[0x26, 0xff]).is_err());
        assert!(report_lengths(&[0xb4]).is_err());
        assert!(report_lengths(&[0x75, 32, 0x95, 255, 0x81, 2]).is_err());
    }
    #[test]
    fn abi_sizes() {
        assert_eq!(size_of::<Create>(), 6196);
        assert_eq!(size_of::<Status>(), 48);
    }
}
