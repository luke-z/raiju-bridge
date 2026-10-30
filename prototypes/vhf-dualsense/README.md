# Raiju VHF controller experiments

**Confirmed working in Destiny 2 on 2026-10-30:** driver `0.1.0.1` with
`run dualsense`, Raiju in PS mode, and Steam Input disabled. The user confirmed
that physical controls work and PlayStation glyphs appear. This path uses no
VIIPER or USB/IP. The Rust forwarding process must remain running; the GPUI
application is not required. The native `raiju-tray.exe` now hosts forwarding
directly, with no console or helper process.

Detection and basic controls are confirmed. Full button/touchpad validation,
latency and production packaging remain outstanding. Toggle/exit/restart have
passed live driver tests. The user confirmed physical PC-to-PS mode switching
returns to green automatically. Suspend/resume and disconnect transitions also
have an automated backend test; a full physical sleep/resume test is still needed.

Driver `0.1.0.2` is a signing-only revision of the working `0.1.0.1` driver.
Its driver code is unchanged; the client now includes the tray runtime. At the user's request, the replacement local
code-signing certificate is valid for 100 years, until **2126-10-30**. This is
a finite expiration date, not an unlimited or publicly issued certificate.
The replacement package is installed locally. Its HID input, calibration and
three virtual feature-command checks pass; gameplay confirmation above is from
the code-identical `0.1.0.1` package.

This isolated prototype tests a Raiju V3 Pro in PS mode with progressively more
DualSense-compatible HID behavior. It has no
VIIPER, Go runtime, USB/IP, HidHide, firmware changes, or game hooks. The existing
GPUI bridge is unchanged.

## Runtime and distribution

Open `package/raiju-tray.exe` after the driver has been installed. It has no
settings window. Click its tray icon for **Enabled** and **Quit**; hovering shows
the connection status. It may initially appear in Windows' hidden-icons area.

| Icon | Meaning |
| --- | --- |
| Green | Real PS-mode reports are being forwarded |
| Amber | Waiting for PS mode, PC mode detected, or multiple PS-mode devices |
| Grey | Disabled; no virtual controller |
| Red | Connection failed; retrying automatically |

The toggle is remembered between launches. Disabling or quitting removes the
virtual controller; it does not disable or hide the physical Raiju. Enabling
reconnects automatically. Unplugging, switching modes and driver session loss
are retried without a console or helper process. The app restores its tray icon
after Explorer restarts and handles suspend/resume notifications.

In **PC mode**, the app leaves native XInput alone and creates no virtual
controller. Games therefore use their ordinary Xbox prompts. Switch back to
**PS mode** for DualSense translation and PlayStation prompts. Keep Steam Input
disabled for Destiny. This avoids requiring HidHide for the tray workflow.

The runtime has two pieces: the UMDF driver creates the virtual controller, and
the tray executable reads the physical Raiju and translates its input. GPUI
and `raiju-vhf-probe.exe` are not needed during play. The probe remains available
for diagnostics and uses the same forwarding code. Moving all forwarding into
the driver would be a separate implementation change.

Installation registers the tray at login for the installing user. On an already
installed driver, `package/raiju-tray.exe --install-startup` registers and starts
it without administrator rights. `--remove-startup` unregisters login startup.
The enable flag is stored in `HKCU/Software/RaijuBridge/VhfTray`; runtime errors
are recorded in `%LOCALAPPDATA%/RaijuBridge/bridge.log` (bounded to about 64 KiB).
The diagnostic commands `--on`, `--off`, `--menu` and `--quit` control an existing
instance. Opening the executable again opens its menu instead of another bridge.

Other compatible PCs can use the same signed DLL, catalog and Rust executable.
They do not need their own signing key or certificate. With this self-signed
prototype, each PC must explicitly trust the supplied public `.cer` certificate
in its machine Root and TrustedPublisher stores. Never distribute the private
signing key. See Microsoft's
[driver certificate trust documentation](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/trusted-root-certification-authorities-certificate-store).
The current installer still requires WDK 26100's `devcon.exe`; it is a local
development installer, not a finished installer for other users. A production
installer should remove that tool dependency. A public release should use an
appropriate certificate from a trusted authority, following Microsoft's
[release-signing guidance](https://learn.microsoft.com/en-us/windows-hardware/drivers/develop/signing-a-driver).

## Stick center

The DualSense USB format has 256 values per axis, from 0 through 255. Earlier
controlled game tests found creep in opposite directions at constant 127 and
128, consistent with a neutral point between them; the game's exact calculation
is unverified. The user's VHF test likewise stops drifting at a game deadzone
of `0.01`. Keep that setting: shifting the integer center reverses creep, while
alternating adjacent values would introduce jitter with no guarantee the game
averages them. The bridge adds no deadzone or timed filter. See the
[legacy README's stick-center measurements](../../docs/legacy-gpui.md#stick-center-and-deadzone)
for the controlled comparison.

## Protocol profiles

The C++ UMDF2 driver uses Windows' inbox `VhfUm.dll`, `Vhf.sys`, and `WUDFRd.sys`.
The Rust console client reads the physical Raiju, reconstructs its HID descriptor
through hidapi, caches its advertised readable feature reports, and submits
**unchanged input bytes**. `run sony` presents `054C:0CE6`; `run razer` retains
`1532:1024` (or `1532:1026`). Both keep the physical revision, descriptor and
input unchanged. PnP hardware IDs and HID attributes agree.

The `sony` and `razer` profiles are identity experiments, not DualSense emulation. They do not
invent Sony calibration/firmware replies, forward feature writes, or forward
rumble. VHF also changes the transport/device tree. Failure through VHF alone
cannot prove that no physical-device identity filter could work.

`run dualsense` is the second experiment, using the **same installed driver**.
The client supplies the USB DualSense report descriptor, basic calibration,
pairing and firmware fixtures, and translated input. Core buttons, sticks and
triggers remain one-to-one. Touch contacts move from Raiju offsets 32/36 to
DualSense offsets 33/37, and battery moves from offset 30 to 53 (all offsets
include the report ID). Sequence/timestamp fields are generated; motion is
neutral. No physical firmware is modified. The fixture firmware version and
virtual address describe the compatibility profile, not the Raiju's firmware.
Driver version 0.1.0.1 accepts feature writes within this virtual session and
answers command 80/81 serial, status and sensor-info requests. Those responses
contain prototype fixtures. Full Sony command handling and rumble forwarding
remain absent; nothing is written to the physical Raiju.

## Observed results

- The locally signed UMDF driver loaded with Secure Boot and Memory Integrity
  enabled. No Microsoft driver-signing account was needed for this local UMDF
  test. This does not establish production signing or anti-cheat approval.
- **Identity-only:** Windows and an independent HID reader detected `054C:0CE6`
  and read input successfully. The user reported no response in Destiny after
  a fresh launch with Steam Input disabled.
- The identity-only device caused a **5% warning in Steam**. The raw report had
  byte 53 = `00`, consistent with the standard DualSense battery decoder's
  lowest bucket. Raiju uses an alternate layout: its battery/status byte is 30.
  The sampled byte there was `1B`; the second profile maps this charging/top
  bucket to `1A`. Unavailable/invalid status maps to unknown, not a fabricated
  full battery. These are protocol readings, not an independent battery measurement.
- **Second profile:** independent HID input and calibration feature reads pass,
  but the user reported no response in Destiny. That minimal driver rejected
  all feature writes. During the test its counters rose to 82 feature writes
  and 168 reads; the counters cannot attribute those calls to a process.
  Its original descriptor is 273 bytes;
  hidapi reconstructs it from Windows preparsed data as 437 bytes. The report
  lengths remain input 64, output 48, feature maximum 64.
- DirectInput independently enumerated the virtual device as "DualSense Wireless
  Controller" with matching Sony product GUID and successfully read all four
  axes with no sample errors. Both raw and client-scaled reads worked.
- **Handshake follow-up (driver 0.1.0.1): PASS in Destiny.** Installed without changing the signing
  certificate or OS security settings. Live tests of commands 01 (serial), 03
  (status), and 04 (sensor-info) all returned the expected fixture responses.
  HID input/calibration still pass. After a fresh Destiny launch, the user
  confirmed working physical controls with PlayStation glyphs. The successful
  configuration includes the compatible descriptor/reports and virtual feature
  handshakes; the necessity of each individual handshake was not separately tested.

Evidence lives in `results/identity-only-result.json`,
`results/identity-only-final-verify.json`, `results/dualsense-verify.json`,
`results/dualsense-before-handshake-result.json`, `results/dualsense-directinput.txt`,
`results/virtual-handshake.json`, `results/dualsense-handshake-verify.json`, and
`results/destiny-success.json` (user confirmation, live driver state and exact
package hashes).

## Physical inspection on this PC

The read-only probe found the wired PS-mode Raiju `1532:1024`, revision `0101`:

| Report | ID | Bytes including ID |
| --- | --- | --- |
| Input | 01 | 64 |
| Output | 02 | 48 |
| Feature | 03 | 48 |
| Feature | E0 | 3 |

The reconstructed descriptor is 171 bytes. Unlike a DualSense, this collection
does not advertise Sony feature IDs 05 (calibration), 09 (pairing), or 20
(firmware). This is evidence of a protocol difference, not proof of Destiny's
internal rejection criterion. Results are in `results/physical-probe.json`.

## Build and validation

Requires the already installed VS 2022 C++ Build Tools, WDK/SDK 26100, and Rust.

```powershell
./build.ps1
```

Quit the tray and diagnostic forwarder before rebuilding. Builds a UMDF DLL,
native Rust tray executable and diagnostic EXE, runs the client tests, validates the
INF, generates the catalog, and signs the local package. It creates/reuses a
code-signing-only certificate in CurrentUser/My. Its lifetime defaults to the
user-selected 100 years; `-CertificateYears` can select 1 through 100 years.
The builder reuses only a certificate with a matching lifetime. It does **not** trust
that certificate, install any device, or change boot/security configuration.
The private key is non-exportable; `package/RaijuVhf.cer` is the public part.
Changing certificate lifetime requires a new certificate and re-signing the
DLL/catalog; an existing certificate's expiry cannot be edited.
The build disables the Spectre-library requirement because those build-only
libraries are absent; this does not disable an OS mitigation. This private
prototype is not a production driver release.

Additional checks:

```powershell
cargo clippy --locked --manifest-path client/Cargo.toml --release --all-targets -- -D warnings
```

## Install on the local test host

Close Destiny, the tray, and the old Raiju Bridge first. Installation needs administrator rights and
explicit approval to trust the prototype's local signer. It adds only the
public certificate from the verified package to LocalMachine/Root and
LocalMachine/TrustedPublisher, and creates `ROOT\RAIJUVHFPROTOTYPE`. Its INF
matches no physical controller. The install script records exactly which
certificate-store entries it added for removal later.

```powershell
./install.ps1 -Action Install
```

Then open `package/raiju-tray.exe` once; it starts automatically at later logins.
The current installer is still a development script requiring WDK's DevCon,
not a finished public installer. Runtime requires only the installed driver and
the tray executable, not the WDK or Rust compiler.

For a rebuilt driver with a newer INF version, close the game/client and use
`./install.ps1 -Action Update` in an elevated PowerShell. This preserves the
record of certificate trust and older prototype OEM packages for cleanup.

Changing signer requires removing the old owned trust/device and installing
the new package. `renew-signing.ps1` performs this local rotation and can roll
back to the preserved `outputs/RaijuVhfPrototype-0.1.0.1/package` snapshot if
installation fails. It is specific to this prototype/workspace. It removes
the old certificate only from the machine stores recorded as added by this
prototype; it leaves the old personal signing certificate for rollback.

A longer-lived certificate extends local Windows trust. It does not establish
Bungie approval or guarantee future Windows compatibility. The prototype uses
ordinary physical-controller input and does not inject game code, automate
gameplay, reduce recoil, or convert mouse input into controller aim assist.
Bungie's [current enforcement policy](https://help.bungie.net/hc/en-us/articles/360049517431-Destiny-Account-Restrictions-and-Banning-Policies)
does not explicitly approve this custom driver.

The UMDF path avoids a newly written kernel binary. This host accepted the
locally signed package. The installer stops if another host rejects it. It never disables Secure Boot, Memory Integrity,
signature enforcement, or anti-cheat, and never enables test-signing mode.
No HID gamepad child exists until `run` opens a session.

## Detection test

1. Keep the Raiju in PS mode, the existing bridge closed, and Steam Input
   **disabled for Destiny**. Restart Destiny for each identity comparison.
2. Run `./package/raiju-vhf-probe.exe run sony` for identity-only, or
   `./package/raiju-vhf-probe.exe run dualsense` for the second profile, and keep it running.
3. Verify a new `054C:0CE6` HID gamepad appears in Windows. Before launching the
   game, independently check it with Windows Game Controllers (`joy.cpl`) or a
   HID reader. Windows enumeration alone is not game compatibility.
4. Launch Destiny and test physical buttons/sticks plus in-game prompts. Do not
   infer game detection from Steam's controller illustration.
5. Exit Destiny, stop the client with Ctrl+C, then optionally compare
   `run razer` using the same transport and unchanged reports.

The status counters include physical submissions and HID feature/output calls;
they do not identify the requesting process or prove Destiny opened the device.
The original Raiju remains visible, so Steam lists the real Raiju and the virtual
DualSense separately. "Wireless Controller" is Steam's Sony product-family name;
it does not mean the virtual connection is wireless. No physical controller
driver has been replaced or hidden.

The forwarding session removes its child after 200 ms without physical updates; the driver
also has a 250 ms neutral-input watchdog. Closing/crashing the client deletes
its HID child. The original watchdog neutral packet was written for the Raiju
identity-only layout; it is not a full DualSense state reset. This remains a
prototype, not a finished daily-use remapper. The tray retries the connection
after session loss and resume. The diagnostic console still exits on disconnect.
Physical sleep/resume, full controls and production lifecycle validation remain
outstanding; the automatic state transitions have an isolated backend test.

Tray verification is recorded in `results/tray-lifecycle.json`,
`results/tray-input-verify.json`, `results/tray-handshake.json` and
`results/tray-mode-switch.json`. The Windows automation helper did not expose
the notification area/menu as a targetable window, so menu input was exercised
through the app's own commands. The user confirmed the tray icon turns green
again after physically switching PC mode to PS mode.

For an identity comparison, run the two identity-only profiles through the same
VHF transport. A failed native game test still does not isolate the transport
from enumeration, descriptor, feature and report behavior.

## Remove

Stop the client and game, then run in an elevated PowerShell:

```powershell
./install.ps1 -Action Remove
```

This removes only this prototype's root device, its associated OEM package, and
the certificate trust entries recorded as newly added. It also quits the tray
and unregisters its per-user login startup. The signing certificate
in CurrentUser/My remains available to rebuild; it is not machine-trusted after
removal. Installation/removal logs are in `results/`.

## References

- [Microsoft VHF_CONFIG, including its UMDF file handle](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/vhf/ns-vhf-_vhf_config)
- [Microsoft discussion confirming UMDF VHF support](https://github.com/microsoft/DMF/issues/273)
- [Nonary/libvirtualgamepad](https://github.com/Nonary/libvirtualgamepad), MIT:
  reference for the UMDF/VHF INF stack and local-target setup; this prototype
  does not bundle or depend on that driver.
- [Physical DualSense USB descriptor](https://github.com/nondebug/dualsense/blob/main/report-descriptor-usb.txt)

Project source is GPL-3.0-or-later, as in the parent repository. No Sony driver
or Sony firmware is redistributed.
