use super::*;

pub(super) fn inspect_touchpad(args: &[String]) -> Result<()> {
    let api = hidapi::HidApi::new()?;
    let mut reports = Vec::new();
    for d in api.device_list().filter(|d| {
        d.vendor_id() == 0x1532
            && matches!(d.product_id(), 0x1025 | 0x1027)
            && (d.usage_page() == 0x0d || d.usage_page() >= 0xff00)
    }) {
        let mut row = serde_json::json!({"path":d.path().to_string_lossy(),"usage_page":d.usage_page(),"usage":d.usage()});
        match d.open_device(&api) {
            Ok(input) => {
                let mut descriptor = [0u8; 4096];
                match input.get_report_descriptor(&mut descriptor) {
                    Ok(n) => row["descriptor"] = serde_json::json!(&descriptor[..n]),
                    Err(e) => row["descriptor_error"] = serde_json::json!(e.to_string()),
                }
            }
            Err(e) => row["open_error"] = serde_json::json!(e.to_string()),
        }
        reports.push(row);
    }
    let json = serde_json::to_vec_pretty(&reports)?;
    if let Some(path) = args.get(2) {
        std::fs::write(path, json)?;
    } else {
        println!("{}", String::from_utf8(json)?);
    }
    Ok(())
}

pub(super) fn inspect_source(_args: &[String]) -> Result<()> {
    let api = hidapi::HidApi::new()?;
    for d in api
        .device_list()
        .filter(|d| d.vendor_id() == 0x1532 && matches!(d.product_id(), 0x1024..=0x1027))
    {
        println!(
            "{:04x}:{:04x} usage={:04x}:{:04x} interface={} path={}",
            d.vendor_id(),
            d.product_id(),
            d.usage_page(),
            d.usage(),
            d.interface_number(),
            d.path().to_string_lossy()
        );
        if (d.usage_page() == 1 && matches!(d.usage(), 4 | 5)) || d.usage_page() >= 0xff00 {
            match d.open_device(&api) {
                Ok(input) => {
                    let mut b = [0u8; 512];
                    let start = Instant::now();
                    let mut count = 0;
                    while start.elapsed() < Duration::from_secs(2) {
                        let n = input.read_timeout(&mut b, 100)?;
                        if n > 0 {
                            count += 1;
                            if count <= 4 {
                                println!("report {n}: {:02x?}", &b[..n]);
                            }
                        }
                    }
                    println!("{} reports / {:.3} s", count, start.elapsed().as_secs_f64());
                }
                Err(e) => println!("Open failed: {e}"),
            }
        }
    }
    Ok(())
}

pub(super) fn probe(_args: &[String]) -> Result<()> {
    let _exclusive = native::ExclusiveSession::acquire()?;
    let (mut input, name) = raiju_bridge::input::Source::open()?;
    let mut buffer = [0u8; 128];
    let frame = input.read(&mut buffer)?.context("No input received")?;
    let s = frame.state;
    println!(
        "Raiju {name}: buttons={:#x}; axes={},{},{},{}",
        protocol::signature(&s),
        s.lx,
        s.ly,
        s.rx,
        s.ry
    );
    Ok(())
}

pub(super) fn input_rate(args: &[String]) -> Result<()> {
    let _exclusive = native::ExclusiveSession::acquire()?;
    let seconds: u64 = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(10);
    let (mut input, name) = raiju_bridge::input::Source::open()?;
    let mut buffer = [0u8; 128];
    let started = Instant::now();
    let cpu_start = cpu_seconds()?;
    let mut count = 0;
    let mut fresh = 0;
    let mut intervals = Vec::new();
    let mut last = None;
    while started.elapsed() < Duration::from_secs(seconds) {
        if let Some(frame) = input.read(&mut buffer)? {
            count += 1;
            fresh += usize::from(frame.fresh);
            if let Some(last) = last {
                intervals.push(frame.received.duration_since(last).as_secs_f64() * 1000.0);
            }
            last = Some(frame.received);
        }
    }
    let elapsed = started.elapsed().as_secs_f64();
    let cpu = cpu_seconds()? - cpu_start;
    let cpus = std::thread::available_parallelism()?.get();
    let report = serde_json::json!({"source":name,"seconds":elapsed,"samples":count,"sample_rate_hz":count as f64/elapsed,"new_states":fresh,"new_states_hz":fresh as f64/elapsed,"sample_intervals_ms":measure::stats(intervals),"cpu_seconds":cpu,"logical_cpus":cpus,"cpu_percent_one_logical_core":cpu/elapsed*100.0,"cpu_percent_whole_machine":cpu/elapsed*100.0/cpus as f64,"note":"PC sampling rate is not USB polling-rate verification; new states depend on physical movement. CPU covers the source-polling process, without GPUI or the virtual-device backend."});
    println!("{}", serde_json::to_string_pretty(&report)?);
    if let Some(path) = args.get(3) {
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    Ok(())
}

pub(super) fn benchmark(args: &[String]) -> Result<()> {
    let directory = PathBuf::from(args.get(2).context("Specify result directory")?);
    let count = args.get(3).map(|s| s.parse()).transpose()?.unwrap_or(2000);
    let runs = args.get(4).map(|s| s.parse()).transpose()?.unwrap_or(3);
    let flush = args.get(5).map(|s| s.parse()).transpose()?.unwrap_or(1);
    measure::benchmark(
        &directory,
        count,
        runs,
        flush,
        Arc::new(AtomicBool::new(false)),
        |s| println!("{s}"),
    )?;
    Ok(())
}

pub(super) fn cancel_test(args: &[String]) -> Result<()> {
    let root = PathBuf::from(args.get(2).context("Specify result directory")?);
    let mut rows = Vec::new();
    for mode in [
        Mode::Bridge,
        Mode::Buttons,
        Mode::Benchmark,
        Mode::Sticks,
        Mode::Sweep,
    ] {
        for delay_ms in [0, 5, 100, 400] {
            let before = native::sony_paths()?;
            let mut worker = Worker::start(mode, root.join(format!("{}-{delay_ms}", mode.name())));
            std::thread::sleep(Duration::from_millis(delay_ms));
            let stopped = Instant::now();
            worker.stop();
            let terminal = loop {
                let _: Vec<_> = worker.receiver.try_iter().collect();
                if let Some(outcome) = worker.take_outcome() {
                    break outcome;
                }
                if stopped.elapsed() > Duration::from_secs(20) {
                    bail!("Cancellation took over 20 seconds");
                }
                std::thread::sleep(Duration::from_millis(10));
            };
            if let Outcome::Failed(error) = terminal {
                bail!("{} stop at {delay_ms} ms: {error}", mode.name());
            }
            let elapsed = stopped.elapsed().as_secs_f64() * 1000.0;
            drop(worker);
            let deadline = Instant::now() + Duration::from_secs(3);
            // Windows may finish removing a device from the preceding test.
            // Only devices added by this test can be its leftover devices.
            let mut after = native::sony_paths()?;
            while !after.is_subset(&before) {
                if Instant::now() > deadline {
                    bail!("Virtual device remained after stop");
                }
                std::thread::sleep(Duration::from_millis(50));
                after = native::sony_paths()?;
            }
            println!(
                "{} +{delay_ms} ms: stopped in {elapsed:.1} ms, no leftover device",
                mode.name()
            );
            rows.push(serde_json::json!({"mode":mode.name(),"stop_after_ms":delay_ms,"stop_duration_ms":elapsed,"leftover_devices":0,"preexisting_devices":before.len(),"removed_baseline_devices":before.difference(&after).count()}));
        }
    }
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join("cancellation-results.json"),
        serde_json::to_vec_pretty(&rows)?,
    )?;
    Ok(())
}

pub(super) fn diagnostics_test(args: &[String]) -> Result<()> {
    let root = PathBuf::from(args.get(2).context("Specify result directory")?);
    let mut worker = Worker::start(Mode::Bridge, root.clone());
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(Message::Status(status)) =
            worker.receiver.recv_timeout(Duration::from_millis(100))
            && status.starts_with("Connected")
        {
            break;
        }
        if let Some(Outcome::Failed(error)) = worker.take_outcome() {
            bail!(error);
        }
        if Instant::now() > deadline {
            worker.stop();
            bail!("Bridge did not connect");
        }
    }
    let devices = native::sony_paths()?;
    let mut rows = Vec::new();
    for enabled in [false, true, false, true] {
        worker.diagnostics(enabled);
        let start = Instant::now();
        let cpu = cpu_seconds()?;
        let mut messages = 0;
        let mut stable_messages = 0;
        let mut rate = 0.0;
        let mut snapshot = None;
        while start.elapsed() < Duration::from_secs(5) {
            match worker.receiver.recv_timeout(Duration::from_millis(100)) {
                Ok(Message::Diagnostics(s)) => {
                    messages += 1;
                    if start.elapsed() > Duration::from_secs(1) {
                        stable_messages += 1;
                    }
                    if let Some(error) = &s.reader_error {
                        bail!("Diagnostics: {error}");
                    }
                    snapshot = Some(s);
                }
                Ok(Message::Reports(_, hz, _, _)) => rate = hz,
                _ => {}
            }
            if let Some(outcome) = worker.take_outcome() {
                bail!("Bridge stopped during toggle: {outcome:?}");
            }
        }
        let seconds = start.elapsed().as_secs_f64();
        let cpu_percent = (cpu_seconds()? - cpu) / seconds * 100.0;
        if native::sony_paths()? != devices {
            bail!("Toggling diagnostics replaced the virtual device");
        }
        if enabled && stable_messages == 0 {
            bail!("Live diagnostics produced no updates");
        }
        if !enabled && stable_messages != 0 {
            bail!("Diagnostic reader continued while disabled");
        }
        let row = serde_json::json!({"enabled":enabled,"seconds":seconds,"cpu_percent_one_logical_core":cpu_percent,"polls_per_second":rate,"snapshot_messages":messages,"messages_after_settling":stable_messages,"same_virtual_device":true,"snapshot":snapshot});
        println!("{}", serde_json::to_string(&row)?);
        rows.push(row);
    }
    worker.stop();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(outcome) = worker.take_outcome() {
            if let Outcome::Failed(error) = outcome {
                bail!("Diagnostics shutdown: {error}");
            }
            break;
        }
        if Instant::now() >= deadline {
            bail!("Diagnostics shutdown took over 20 seconds");
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    std::fs::create_dir_all(&root)?;
    std::fs::write(
        root.join("diagnostics-toggle.json"),
        serde_json::to_vec_pretty(&rows)?,
    )?;
    Ok(())
}
