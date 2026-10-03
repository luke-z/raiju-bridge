#[path = "cli/commands.rs"]
mod commands;
use anyhow::{Context, Result, bail};
use raiju_bridge::{
    accuracy, measure, native, protocol,
    worker::{self, Message, Mode, Outcome, Worker},
};
use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool, mpsc::RecvTimeoutError},
    time::{Duration, Instant},
};
fn main() {
    if raiju_bridge::hiding::watchdog_entry() {
        return;
    }
    if raiju_bridge::touchpad::watchdog_entry() {
        return;
    }
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}
fn cpu_seconds() -> Result<f64> {
    use windows_sys::Win32::{
        Foundation::FILETIME,
        System::Threading::{GetCurrentProcess, GetProcessTimes},
    };
    let mut created = FILETIME::default();
    let mut exited = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    if unsafe {
        GetProcessTimes(
            GetCurrentProcess(),
            &mut created,
            &mut exited,
            &mut kernel,
            &mut user,
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error().into());
    }
    let ticks = |v: FILETIME| ((v.dwHighDateTime as u64) << 32) | v.dwLowDateTime as u64;
    Ok((ticks(kernel) + ticks(user)) as f64 / 10_000_000.0)
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    match args.get(1).map(String::as_str) {
        Some("--touch-sweep") => raiju_bridge::touchpad::validation::sweep(
            &args
                .get(2)
                .map(PathBuf::from)
                .unwrap_or_else(|| worker::results_root().join("touch-sweep")),
        )?,
        Some("--inspect-touchpad") => commands::inspect_touchpad(&args)?,

        Some("--inspect-source") => commands::inspect_source(&args)?,

        Some("--probe") => commands::probe(&args)?,

        Some("--input-rate") => commands::input_rate(&args)?,

        Some("--bench") => commands::benchmark(&args)?,

        Some("--stick-sweep") => {
            let directory = PathBuf::from(args.get(2).context("Specify result directory")?);
            println!(
                "{:?}",
                accuracy::sweep(&directory, &AtomicBool::new(false), |s| println!("{s}"))?
            );
        }
        Some("--cancel-test") => commands::cancel_test(&args)?,

        Some("--diagnostics-test") => commands::diagnostics_test(&args)?,

        Some(command @ ("--run" | "--monitor" | "--buttons" | "--sticks")) => {
            let mode = match command {
                "--run" | "--monitor" => Mode::Bridge,
                "--sticks" => Mode::Sticks,
                _ => Mode::Buttons,
            };
            let seconds: u64 = args.get(2).map(|s| s.parse()).transpose()?.unwrap_or(20);
            let directory = args
                .get(3)
                .map(PathBuf::from)
                .unwrap_or_else(|| worker::session_directory(mode));
            let mut worker = Worker::start(mode, directory);
            worker.diagnostics(command == "--monitor");
            let deadline = Instant::now() + Duration::from_secs(seconds);
            loop {
                if Instant::now() >= deadline {
                    worker.stop();
                }
                match worker.receiver.recv_timeout(Duration::from_millis(100)) {
                    Ok(Message::Status(s)) => println!("{s}"),
                    Ok(Message::Connected(s)) => println!("Raiju {s}"),
                    Ok(Message::Reports(n, hz, fresh, pc)) => println!(
                        "{n} {} · {hz:.1}/s · {fresh:.1} fresh/s",
                        if pc { "polls" } else { "reports" }
                    ),
                    Ok(Message::Button(s)) => println!("{} · {:.4} ms", s.action, s.delay_ms),
                    Ok(Message::Sticks(s)) => println!(
                        "Input {:?} / Output {:?} · {} matched, {} skipped, {} unmatched",
                        s.input, s.output, s.matched, s.skipped, s.unexpected
                    ),
                    Ok(Message::Diagnostics(s)) => println!(
                        "Live: {} buttons, {} stick changes; reader error {:?}",
                        s.button_matches, s.sticks.matched, s.reader_error
                    ),
                    Err(RecvTimeoutError::Timeout) => {}
                    Err(RecvTimeoutError::Disconnected) => {}
                }
                if let Some(outcome) = worker.take_outcome() {
                    match outcome {
                        Outcome::Failed(e) => bail!(e),
                        other => {
                            println!("{other:?}");
                            break;
                        }
                    }
                }
            }
        }
        _ => println!(
            "Raiju Bridge\n  --probe\n  --input-rate [seconds] [json_path]\n  --run [seconds]\n  --buttons [seconds] [directory]\n  --sticks [seconds] [directory]\n  --stick-sweep <directory>\n  --cancel-test <directory>\n  --bench <directory> [samples_per_run=2000] [runs=3] [flush_ms=1]"
        ),
    }
    Ok(())
}
