#![windows_subsystem = "windows"]

use anyhow::{Context, Result, ensure};
use raiju_vhf_probe::runtime::{BridgeRuntime, State, log};
use std::{
    cell::{Cell, OnceCell},
    mem::size_of,
    ptr,
};
use windows_sys::{
    Win32::{
        Foundation::*,
        System::{LibraryLoader::GetModuleHandleW, Registry::*, Threading::CreateMutexW},
        UI::{Shell::*, WindowsAndMessaging::*},
    },
    core::{GUID, w},
};

const CLASS: windows_sys::core::PCWSTR = w!("RaijuVhfTray.Window");
const NAME: windows_sys::core::PCWSTR = w!("Raiju Bridge");
const PREFS: windows_sys::core::PCWSTR = w!("Software\\RaijuBridge\\VhfTray");
const STARTUP_NAME: windows_sys::core::PCWSTR = w!("RaijuVhfTray");
const RUN: windows_sys::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const ICON_GUID: GUID = GUID::from_u128(0x8c26d584_40b1_45e8_b6ef_6d48c38a9f91);
const TRAY: u32 = WM_APP + 1;
const CHANGED: u32 = WM_APP + 2;
const TOGGLE: usize = 1001;
const QUIT: usize = 1002;
const ON: usize = 1003;
const OFF: usize = 1004;
const MENU: usize = 1005;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

struct Icons(Vec<HICON>);
impl Icons {
    fn new() -> Result<Self> {
        let mut icons = Self(Vec::new());
        // Green = forwarding, amber = waiting, grey = disabled, red = error.
        for [red, green, blue] in [
            [42, 196, 116],
            [235, 170, 48],
            [134, 143, 156],
            [228, 83, 83],
        ] {
            let mut mask = [255u8; 128];
            let mut pixels = [0u8; 4096];
            for y in 0..32 {
                for x in 0..32 {
                    let (dx, dy) = (x as f32 - 15.5, y as f32 - 15.5);
                    let radius = dx.hypot(dy);
                    let alpha = ((14.5 - radius).clamp(0.0, 1.0) * 255.0) as u8;
                    let power = ((radius - 7.0).abs() < 1.3 && (dy > -3.0 || dx.abs() > 3.0))
                        || (dx.abs() < 1.3 && (-9.0..=-1.0).contains(&dy));
                    let [r, g, b] = if power {
                        [255, 255, 255]
                    } else {
                        [red, green, blue]
                    };
                    let offset = ((31 - y) * 32 + x) * 4;
                    for (index, value) in [b, g, r].iter().enumerate() {
                        pixels[offset + index] = (*value as u16 * alpha as u16 / 255) as u8;
                    }
                    pixels[offset + 3] = alpha;
                    if alpha > 0 {
                        mask[(31 - y) * 4 + x / 8] &= !(128 >> (x % 8));
                    }
                }
            }
            let icon = unsafe {
                CreateIcon(
                    ptr::null_mut(),
                    32,
                    32,
                    1,
                    32,
                    mask.as_ptr(),
                    pixels.as_ptr(),
                )
            };
            ensure!(!icon.is_null(), "Cannot create tray icon");
            icons.0.push(icon);
        }
        Ok(icons)
    }
}
impl Drop for Icons {
    fn drop(&mut self) {
        for &icon in &self.0 {
            unsafe {
                DestroyIcon(icon);
            }
        }
    }
}

// Window callbacks may reenter while a popup is open. Use shared references and
// interior mutability rather than creating overlapping mutable references.
struct App {
    hwnd: Cell<HWND>,
    runtime: OnceCell<BridgeRuntime>,
    icons: Icons,
    taskbar_created: u32,
}
impl App {
    fn data(&self) -> NOTIFYICONDATAW {
        NOTIFYICONDATAW {
            cbSize: size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: self.hwnd.get(),
            uID: 1,
            guidItem: ICON_GUID,
            uFlags: NIF_GUID,
            ..Default::default()
        }
    }
    fn redraw(&self, add: bool) -> Result<()> {
        let state = self
            .runtime
            .get()
            .map_or(State::Waiting, BridgeRuntime::state);
        let index = match state {
            State::Running => 0,
            State::Off => 2,
            State::Error => 3,
            _ => 1,
        };
        let mut data = self.data();
        data.uFlags |= NIF_MESSAGE | NIF_ICON | NIF_TIP | NIF_SHOWTIP;
        data.uCallbackMessage = TRAY;
        data.hIcon = self.icons.0[index];
        for (slot, value) in data
            .szTip
            .iter_mut()
            .take(127)
            .zip(format!("Raiju Bridge - {}", state.label()).encode_utf16())
        {
            *slot = value;
        }
        unsafe {
            let mut added = add;
            if Shell_NotifyIconW(if add { NIM_ADD } else { NIM_MODIFY }, &data) == 0 {
                ensure!(
                    !add && Shell_NotifyIconW(NIM_ADD, &data) != 0,
                    "Cannot add tray icon"
                );
                added = true;
            }
            if added {
                data.Anonymous.uVersion = NOTIFYICON_VERSION_4;
                ensure!(
                    Shell_NotifyIconW(NIM_SETVERSION, &data) != 0,
                    "Cannot configure tray icon"
                );
            }
        }
        Ok(())
    }
    fn command(&self, command: usize) {
        let Some(runtime) = self.runtime.get() else {
            return;
        };
        match command {
            TOGGLE | ON | OFF => {
                let enabled = if command == TOGGLE {
                    !runtime.enabled()
                } else {
                    command == ON
                };
                // Save before changing live input, so a failed save is reported.
                if let Err(error) = save_enabled(enabled) {
                    log(&format!("Save toggle: {error:#}"));
                }
                runtime.set_enabled(enabled);
            }
            QUIT => unsafe {
                DestroyWindow(self.hwnd.get());
            },
            MENU => self.menu(),
            _ => (),
        }
    }
    fn menu(&self) {
        let Some(runtime) = self.runtime.get() else {
            return;
        };
        unsafe {
            let menu = CreatePopupMenu();
            if menu.is_null() {
                return;
            }
            AppendMenuW(
                menu,
                MF_STRING | MF_DISABLED,
                0,
                wide(runtime.state().label()).as_ptr(),
            );
            AppendMenuW(menu, MF_SEPARATOR, 0, ptr::null());
            AppendMenuW(
                menu,
                MF_STRING
                    | if runtime.enabled() {
                        MF_CHECKED
                    } else {
                        MF_UNCHECKED
                    },
                TOGGLE,
                w!("Enabled"),
            );
            AppendMenuW(menu, MF_STRING, QUIT, w!("Quit"));
            let mut point = POINT::default();
            GetCursorPos(&mut point);
            SetForegroundWindow(self.hwnd.get());
            let command = TrackPopupMenu(
                menu,
                TPM_RETURNCMD | TPM_NONOTIFY | TPM_RIGHTBUTTON | TPM_BOTTOMALIGN,
                point.x,
                point.y,
                0,
                self.hwnd.get(),
                ptr::null(),
            );
            DestroyMenu(menu);
            PostMessageW(self.hwnd.get(), WM_NULL, 0, 0);
            Shell_NotifyIconW(NIM_SETFOCUS, &self.data());
            self.command(command as usize);
        }
    }
}

unsafe extern "system" fn window_proc(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        if message == WM_NCCREATE {
            let create = &*(lp as *const CREATESTRUCTW);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, create.lpCreateParams as isize);
        }
        let Some(app) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const App).as_ref() else {
            return DefWindowProcW(hwnd, message, wp, lp);
        };
        if message == app.taskbar_created {
            if let Err(error) = app.redraw(true) {
                log(&format!("Restore icon: {error:#}"));
            }
            return 0;
        }
        match message {
            TRAY => {
                // Version 4 packs the notification code in the low word.
                if matches!(lp as u32 & 0xffff, NIN_SELECT | 1025 | WM_CONTEXTMENU) {
                    app.menu();
                }
                0
            }
            CHANGED => {
                if let Err(error) = app.redraw(false) {
                    log(&format!("Update icon: {error:#}"));
                }
                0
            }
            WM_COMMAND => {
                app.command(wp & 0xffff);
                0
            }
            WM_DEVICECHANGE => {
                if let Some(r) = app.runtime.get() {
                    r.wake();
                }
                0
            }
            WM_POWERBROADCAST => {
                if let Some(runtime) = app.runtime.get() {
                    match wp as u32 {
                        PBT_APMSUSPEND => runtime.suspend(true),
                        PBT_APMRESUMEAUTOMATIC | PBT_APMRESUMESUSPEND | PBT_APMRESUMECRITICAL => {
                            runtime.suspend(false)
                        }
                        _ => (),
                    }
                }
                1
            }
            WM_QUERYENDSESSION => 1,
            WM_ENDSESSION if wp != 0 => {
                DestroyWindow(hwnd);
                0
            }
            WM_CLOSE => {
                DestroyWindow(hwnd);
                0
            }
            WM_DESTROY => {
                if let Some(runtime) = app.runtime.get() {
                    runtime.stop();
                }
                Shell_NotifyIconW(NIM_DELETE, &app.data());
                PostQuitMessage(0);
                0
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                DefWindowProcW(hwnd, message, wp, lp)
            }
            _ => DefWindowProcW(hwnd, message, wp, lp),
        }
    }
}

fn saved_enabled() -> bool {
    let mut value = 1u32;
    let mut length = size_of::<u32>() as u32;
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PREFS,
            w!("Enabled"),
            RRF_RT_REG_DWORD,
            ptr::null_mut(),
            (&mut value as *mut u32).cast(),
            &mut length,
        )
    };
    status != ERROR_SUCCESS || value != 0
}
fn save_enabled(enabled: bool) -> Result<()> {
    let value = u32::from(enabled);
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            PREFS,
            w!("Enabled"),
            REG_DWORD,
            (&value as *const u32).cast(),
            size_of::<u32>() as u32,
        )
    };
    ensure!(status == ERROR_SUCCESS, "Cannot save Enabled: {status}");
    Ok(())
}
fn startup(install: bool) -> Result<()> {
    let status = if install {
        let path = std::env::current_exe()?;
        let command = wide(&format!("\"{}\"", path.display()));
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                RUN,
                STARTUP_NAME,
                REG_SZ,
                command.as_ptr().cast(),
                (command.len() * 2) as u32,
            )
        }
    } else {
        unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, RUN, STARTUP_NAME) }
    };
    ensure!(
        status == ERROR_SUCCESS || (!install && status == ERROR_FILE_NOT_FOUND),
        "Cannot update login startup: {status}"
    );
    Ok(())
}

struct Instance(HANDLE);
impl Drop for Instance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.0) != 0 {
                DestroyWindow(self.0);
            }
        }
    }
}

fn run() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("--install-startup") => startup(true)?,
        Some("--register-startup") => return startup(true),
        Some("--remove-startup") => return startup(false),
        Some(argument @ ("--on" | "--off" | "--quit" | "--menu")) => {
            let hwnd = unsafe { FindWindowW(CLASS, ptr::null()) };
            ensure!(!hwnd.is_null(), "Raiju Bridge is not running");
            let command = match argument {
                "--on" => ON,
                "--off" => OFF,
                "--menu" => MENU,
                _ => QUIT,
            };
            ensure!(
                unsafe { PostMessageW(hwnd, WM_COMMAND, command, 0) } != 0,
                "Cannot send tray command"
            );
            return Ok(());
        }
        Some(_) => anyhow::bail!("Unknown argument"),
        None => (),
    }
    let mutex = unsafe { CreateMutexW(ptr::null(), 0, w!("Local\\RaijuVhfTray.Instance")) };
    ensure!(!mutex.is_null(), "Cannot create instance lock");
    let duplicate = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let _instance = Instance(mutex);
    if duplicate {
        let hwnd = unsafe { FindWindowW(CLASS, ptr::null()) };
        if !hwnd.is_null() {
            unsafe {
                PostMessageW(hwnd, WM_COMMAND, MENU, 0);
            }
        }
        return Ok(());
    }
    let mut app = Box::new(App {
        hwnd: Cell::new(ptr::null_mut()),
        runtime: OnceCell::new(),
        icons: Icons::new()?,
        taskbar_created: unsafe { RegisterWindowMessageW(w!("TaskbarCreated")) },
    });
    ensure!(
        app.taskbar_created != 0,
        "Cannot register taskbar restart message"
    );
    unsafe {
        let module = GetModuleHandleW(ptr::null());
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: module,
            lpszClassName: CLASS,
            hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
            ..Default::default()
        };
        ensure!(RegisterClassW(&class) != 0, "Cannot register tray window");
        // Hidden top-level window receives power and Explorer restart broadcasts.
        let hwnd = CreateWindowExW(
            WS_EX_TOOLWINDOW,
            CLASS,
            NAME,
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            ptr::null_mut(),
            ptr::null_mut(),
            module,
            (&mut *app as *mut App).cast(),
        );
        ensure!(!hwnd.is_null(), "Cannot create tray window");
        let _window = Window(hwnd);
        app.hwnd.set(hwnd);
        let address = hwnd as usize;
        let runtime = BridgeRuntime::start(saved_enabled(), move || {
            PostMessageW(address as HWND, CHANGED, 0, 0);
        })?;
        ensure!(
            app.runtime.set(runtime).is_ok(),
            "Tray runtime already initialized"
        );
        if let Err(error) = app.redraw(true) {
            DestroyWindow(hwnd);
            return Err(error);
        }
        let mut message = MSG::default();
        loop {
            let result = GetMessageW(&mut message, ptr::null_mut(), 0, 0);
            if result == 0 {
                break;
            }
            if result == -1 {
                DestroyWindow(hwnd);
                anyhow::bail!("Tray message loop failed");
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    // Drop joins the input worker, releasing the virtual controller before exit.
    Ok(())
}

fn main() {
    if let Err(error) = run().context("Raiju Bridge") {
        log(&format!("{error:#}"));
        unsafe {
            MessageBoxW(
                ptr::null_mut(),
                wide(&format!("{error:#}")).as_ptr(),
                NAME,
                MB_OK | MB_ICONERROR,
            );
        }
        std::process::exit(1);
    }
}
