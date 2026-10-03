# Building and checks

Use the Rust version pinned in `rust-toolchain.toml` and Python 3.11+.

## Windows

Install Visual Studio C++ Build Tools and the Windows SDK, then run:

```powershell
python scripts/bootstrap.py
./build.ps1
```

The executables are in `target/release`. Obtain `libVIIPER.dll` from the pinned [VIIPER v0.8.2 Windows amd64 library](https://github.com/Alia5/VIIPER/releases/tag/v0.8.2), or run the packaging script described below. The GUI needs the DLL beside it at runtime; unit tests do not need a controller or driver.

## Ubuntu → Windows x64

The checked-in workflow runs only when a `vX.X.X` tag is pushed (for example, `v0.3.0`). It uses LLVM, `cargo-xwin` and Wine. FXC is Microsoft's shader compiler from a checksum-verified Windows SDK package. Wine runs FXC and the Windows unit tests; Rust and C/C++ compilation run on Linux. All jobs use `ubuntu-latest`.

```sh
python3 scripts/bootstrap.py
python3 scripts/setup_fxc.py
chmod +x scripts/fxc-wine.py
export GPUI_FXC_PATH="$PWD/scripts/fxc-wine.py"
export WINEDEBUG=-all
cargo fmt --all -- --check
cargo xwin clippy --locked --release --all-targets --target x86_64-pc-windows-msvc -- -D warnings
cargo xwin test --locked --release --lib --target x86_64-pc-windows-msvc
cargo xwin build --locked --release --bins --target x86_64-pc-windows-msvc
python3 scripts/package.py target/x86_64-pc-windows-msvc/release
```

GPUI 0.2.2 gates its Windows shader/manifest build on the **host** OS. `bootstrap.py` extracts the exact crate into ignored `.build/gpui`, enables that code on Linux, makes its resource-compiler dependency available, and resolves the manifest resource to an absolute path for LLVM. Runtime GPUI code is unchanged. The source checksum and patch preconditions are checked. Nothing is modified in the global Cargo cache. Revisit this patch when updating GPUI.

`+crt-static` avoids requiring a separate Visual C++ runtime install. Build downloads and generated sources stay in `.build/` and `target/`; binaries and test captures are never committed.

## Linting

```sh
cargo fmt --all -- --check
cargo clippy --locked --release --all-targets -- -D warnings
cargo test --locked --release --lib
```

Clippy's `cognitive_complexity` is denied above **20** in `clippy.toml`. Unlike cyclomatic complexity, this metric also weights nesting. Split responsibilities when it fires; do not raise the threshold or add blanket allowances to make CI pass.

## Hardware validation

Stop the GUI before using the CLI. Synthetic tests generate controller input, so close games first.

```powershell
raiju-bridge-cli.exe --probe
raiju-bridge-cli.exe --input-rate 10 rate.json
raiju-bridge-cli.exe --stick-sweep results/sticks
raiju-bridge-cli.exe --touch-sweep results/touch
raiju-bridge-cli.exe --cancel-test results/cancellation
raiju-bridge-cli.exe --diagnostics-test results/diagnostics
```

Hardware tests are local; hosted CI only runs deterministic unit tests. Live diagnostics use a bounded observation queue, refresh the diagram about 60 times/second, and update latency statistics at most 10 times/second. Brief presses flash for 80 ms in the diagram; this does not hold game input. Diagnostics have measurable overhead and can be disabled without reconnecting.

## PC touchpad

Raw Input reads only the Raiju Precision Touchpad collection. Its firmware emits a 1920 × 1080 grid despite the larger range advertised by its HID descriptor. Preserve those coordinates and up to two contacts, retaining finger slots across report reordering. Touch changes are forwarded independently of XInput packet changes.

Windows' `SPI_SETTOUCHPADPARAMETERS` cursor policy is global. The bridge therefore requires the Raiju to be the sole Precision Touchpad and an external mouse to be present. A separate helper temporarily changes only `allowActiveWhenMousePresent`; EOF on its parent pipe restores the original value even if the parent crashes. It checks the environment every second and stops if another touchpad appears or the mouse disappears. Stop restores the preference without disabling or rebinding any physical device. This does not hide the physical Xbox interface.

The touchpad diagram is schematic, with a wider top. Its projection affects only the display, not forwarded coordinates. The Sony output reader uses the standard report layout; the Raiju PS5 input parser uses the third-party layout.

The title bar, executable and tray share icon resource 1. Regenerate the multi-resolution ICO with `python scripts/make_icon.py` after changing its pixel design; the resource is embedded on both native and cross builds.

## Stopping during attachment

VIIPER's automatic attach issues a synchronous driver IOCTL. `cancel::synchronous_io` uses a temporary helper to request `CancelSynchronousIo` on the calling worker when Stop is pressed. The helper is joined before cleanup or forwarding, including on unwind. It does not terminate threads or unload the Go DLL. Go's exported cgo call stays on that OS thread; recheck that assumption if replacing the backend. Windows may still need time to complete cancellation and remove the device.

References: [Windows cancellation API](https://learn.microsoft.com/en-us/windows/win32/api/ioapiset/nf-ioapiset-cancelsynchronousio), [driver cancellation support](https://github.com/vadimgrn/usbip-win2/releases/tag/v.0.9.6.1), [Go cgo thread affinity](https://go.dev/src/runtime/cgocall.go).
