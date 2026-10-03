# Third-party components

- **VIIPER 0.8.2** — GPL-3.0-or-later, [Alia5/VIIPER](https://github.com/Alia5/VIIPER), commit `3111299d67bacbaa7f6a31d56ea9ae06678f5865`. Built from checksum-verified source with a DualSense USB output descriptor correction in `scripts/build_backend.py`. The original source and reproducible patch/build script are included in the portable ZIP.
- **GPUI 0.2.2** — Apache-2.0, [zed-industries/zed](https://github.com/zed-industries/zed). `scripts/bootstrap.py` documents the build-only cross-compilation patch. The original crate and patch script are included with the packaged source.
- Other Rust dependencies are pinned in `Cargo.lock`. The portable ZIP includes their crate sources and collected license notices in `licenses/RUST-DEPENDENCIES.txt`.
- **USB/IP for Windows** is installed separately from its [official releases](https://github.com/vadimgrn/usbip-win2/releases). No driver is bundled or installed by this application.
- **HidHide** is installed separately for PC mode from its [official releases](https://github.com/nefarius/HidHide/releases). The bridge uses its public control-device API to manage session hiding; the driver and configuration client are not bundled.
- Microsoft's Windows SDK/FXC and the MSVC SDK fetched by cargo-xwin are build tools, subject to their Microsoft licenses. They are not included in the application ZIP.

The XInput extended capability ABI follows the layout documented by [SDL's Windows backend](https://github.com/libsdl-org/SDL/blob/main/src/core/windows/SDL_xinput.h). Sony standard and third-party report layouts are cross-checked against [SDL's PS5 backend](https://github.com/libsdl-org/SDL/blob/main/src/joystick/hidapi/SDL_hidapi_ps5.c).

Razer, Raiju, PlayStation and DualSense are trademarks of their respective owners. This is an independent project.
