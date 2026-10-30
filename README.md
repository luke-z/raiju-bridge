# Raiju Bridge

Use a Razer Raiju V3 Pro with PlayStation button prompts in Destiny 2 on Windows.
The new native version is a small tray app and a UMDF/VHF virtual-controller
driver. There is no settings window, console, GPUI, VIIPER, USB/IP or HidHide
requirement for this workflow.

## Daily use

After the one-time driver installation:

1. Connect one Raiju by USB in **PS mode**.
2. Disable **Steam Input for Destiny 2** so the game receives the virtual DualSense.
3. Open `package/raiju-tray.exe`. Click its tray icon to toggle **Enabled** or **Quit**.

The app starts at login after installation and remembers the enable toggle.
It reconnects automatically when you unplug/reconnect the controller or change
back to PS mode. The icon may initially be under Windows' hidden-icons arrow.

| Icon | Status |
| --- | --- |
| Green | Controller input is being forwarded |
| Amber | Waiting for a PS-mode controller; PC mode also waits here |
| Grey | Disabled |
| Red | Connection problem; retrying automatically |

Disabling or quitting removes the virtual controller and leaves the physical
Raiju available. The tray app hosts forwarding directly; no console helper runs.

**PC mode:** the app leaves native XInput alone, so games use Xbox prompts.
Switch back to PS mode for PlayStation prompts; no app restart is needed.
Steam can list both the physical Raiju and virtual DualSense while enabled.

## One-time setup

The native version is currently a locally built prototype. Older GitHub release
downloads contain the [previous GPUI app](docs/legacy-gpui.md).

From the built `prototypes/vhf-dualsense` folder, close the game and both bridge
apps, then run in an elevated PowerShell:

```powershell
./install.ps1 -Action Install
```

This installs the virtual-controller source driver, trusts the package's public
code-signing certificate, and registers the tray app at login for that user.
Then open `package/raiju-tray.exe` once to start now.

The current development installer requires WDK 26100's `DevCon`; a standalone
public installer has not yet been packaged. Daily use needs only the installed
driver and tray executable. No compiler or WDK runs in the background.

Other users can use the same compiled package and public certificate. They
do not need to generate their own certificate or receive a private signing key.
Each PC must explicitly trust the prototype signer during installation.
The installer does not disable Secure Boot, Memory Integrity, signature
enforcement or anti-cheat, and does not enable Windows test-signing mode.

For build requirements, signing, protocol details, updates and removal, see the
[native driver and tray documentation](prototypes/vhf-dualsense/README.md).

## Validation and limitations

Physical controls and PlayStation prompts were confirmed in Destiny 2 with
Steam Input disabled. Live tray tests cover input, feature handshakes, off/on,
exit cleanup, restart persistence and duplicate launches. Switching PC mode
back to PS mode was also confirmed on the physical controller.

The tray uses the same controller translation as the working console prototype.
Its runtime imports only Windows system DLLs. Rumble, adaptive-trigger feedback
and motion forwarding are not implemented. Wireless operation and full physical
sleep/resume validation remain unverified. A working driver and valid signature
do not establish Bungie approval; see the detailed documentation.

For tiny drift at Destiny's `0.00` deadzone, use **`0.01`**. The bridge adds no
deadzone or response curve. Earlier controlled tests found opposite creep at
constant DualSense axis values of 127 and 128, consistent with a neutral point
between them. See the [stick-center measurements](docs/legacy-gpui.md#stick-center-and-deadzone).

## Development

The native sources are in `prototypes/vhf-dualsense`. On Windows:

```powershell
cargo test --locked --release --manifest-path prototypes/vhf-dualsense/client/Cargo.toml
cargo clippy --locked --release --all-targets --manifest-path prototypes/vhf-dualsense/client/Cargo.toml -- -D warnings
```

The full local package, including its signed driver, is built with
`prototypes/vhf-dualsense/build.ps1`. Quit the tray first. Build artifacts and
local signing material are excluded from Git.

The original GPUI app remains in `src/`; its [legacy instructions](docs/legacy-gpui.md)
and tag-triggered release workflow are preserved.

[GPL-3.0-or-later](LICENSE). No physical firmware changes or game hooks.
