# Raiju Bridge

Turn a Razer Raiju V3 Pro into a virtual DualSense on Windows. Use PC or PS5 mode, with a native Rust/GPUI app, Start/Stop, a tray icon, optional startup, and live button, stick and touchpad diagnostics.

<table>
  <tr><th>Compact</th><th>Diagnostics</th></tr>
  <tr>
    <td valign="top"><img src="docs/images/compact.jpg" width="290" alt="Raiju Bridge compact view with one power button and startup settings"></td>
    <td valign="top"><img src="docs/images/diagnostics.jpg" width="450" alt="Raiju Bridge diagnostics with a controller diagram, trigger bars and input measurements"></td>
  </tr>
</table>

## Install

1. Install the signed [USB/IP driver 0.9.8.1](https://github.com/vadimgrn/usbip-win2/releases/tag/v.0.9.8.1) and restart if requested.
2. Download the artifact from the latest successful [Windows build](https://github.com/luke-z/raiju-bridge/actions/workflows/windows.yml). Extract it and the ZIP inside it.
3. Keep `libVIIPER.dll` beside `raiju-bridge.exe`. Connect one Raiju by USB, open the app, and click the **power button**.

Fresh installs open in compact mode. Use **Diagnostics** or **Ctrl D** to expand the live controller view and measurements. Closing the window keeps the bridge in the tray; **Quit** stops it. Windows sign-in and automatic connection are separate settings.

PC mode requires Windows 11 24H2+, a regular mouse, and no other Precision Touchpad. Optional precise 2,000 Hz polling uses more CPU and is **off by default**; enable it in Diagnostics and restart the bridge to apply it. Touchpad contacts and clicks become PlayStation input while cursor movement is suppressed. The original Windows touchpad preference is restored on Stop or app exit, with a watchdog for crashes. Synapse is not needed to run the bridge; existing onboard mappings still apply.

The physical Xbox input remains visible in PC mode, so game compatibility and PlayStation button prompts depend on the game selecting the virtual controller. Rumble, adaptive-trigger feedback and motion forwarding are not implemented. Wired USB has been tested; wireless support is unverified.

[GPL-3.0-or-later](LICENSE). Uses [VIIPER](https://github.com/Alia5/VIIPER) over localhost USB/IP. No firmware changes or cloud service.
