# Raiju Bridge

Turn a Razer Raiju V3 Pro into a virtual DualSense on Windows. Use PC or PS5 mode, with a native Rust/GPUI app, Start/Stop, a tray icon, optional startup, and live button, stick and touchpad diagnostics.

## Install

1. Install the signed [USB/IP driver 0.9.8.1](https://github.com/vadimgrn/usbip-win2/releases/tag/v.0.9.8.1) and restart if requested.
2. Download the artifact from the latest successful [Windows build](https://github.com/luke-z/raiju-bridge/actions/workflows/windows.yml). Extract it and the ZIP inside it.
3. Keep `libVIIPER.dll` beside `raiju-bridge.exe`. Connect one Raiju by USB, open the app, and click **Start bridge**.

PC mode targets 2,000 input polls/second and requires Windows 11 24H2+, a regular mouse, and no other Precision Touchpad. Touchpad contacts and clicks become PlayStation input while cursor movement is suppressed. The original Windows touchpad preference is restored on Stop or app exit, with a watchdog for crashes. Synapse is not needed to run the bridge; existing onboard mappings still apply.

The physical Xbox input remains visible in PC mode, so game compatibility and PlayStation button prompts depend on the game selecting the virtual controller. Rumble, adaptive-trigger feedback and motion forwarding are not implemented. Wired USB has been tested; wireless support is unverified.

## Develop

See [building and checks](docs/development.md). CI builds Windows x64 on `ubuntu-latest`, runs tests under Wine, checks formatting, and runs Clippy with a **cognitive complexity limit of 20**. This is Rust's nesting-aware metric, rather than Oxlint's JavaScript cyclomatic metric.

[GPL-3.0-or-later](LICENSE). Uses [VIIPER](https://github.com/Alia5/VIIPER) over localhost USB/IP. No firmware changes or cloud service.
