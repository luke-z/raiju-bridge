#![windows_subsystem = "windows"]
mod panel;
mod tray;
use gpui::{
    App, Application, Bounds, Context, FocusHandle, FontWeight, KeyDownEvent, SharedString,
    TitlebarOptions, Window, WindowBounds, WindowOptions, div, prelude::*, px, rgb, size,
};
use raiju_bridge::{
    accuracy::StickSnapshot,
    desktop::{self, AppInstance, Settings},
    measure::{self, Stats},
    worker::{self, Message, Mode, Outcome, Worker},
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use std::{collections::VecDeque, io::Write, time::Duration};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    SW_HIDE, SW_RESTORE, SetForegroundWindow, ShowWindow,
};
const INK: u32 = 0x11191b;
const PANEL: u32 = 0x1b2527;
const LINE: u32 = 0x344143;
const TEXT: u32 = 0xe7f0ed;
const MUTED: u32 = 0x9babaa;
const ACCENT: u32 = 0xd9f27e;

fn visibility(window: &Window, show: bool) {
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        unsafe {
            ShowWindow(
                handle.hwnd.get() as _,
                if show { SW_RESTORE } else { SW_HIDE },
            );
            if show {
                SetForegroundWindow(handle.hwnd.get() as _);
            }
        }
    }
}
struct BridgeUi {
    worker: Option<Worker>,
    mode: Option<Mode>,
    status: String,
    source: String,
    hz: f64,
    fresh_hz: f64,
    pc_mode: bool,
    statistics: Stats,
    values: VecDeque<f64>,
    sticks: Option<StickSnapshot>,
    live: Option<Box<raiju_bridge::diagnostics::Snapshot>>,
    sweep_result: Option<String>,
    logs: VecDeque<String>,
    stopping: bool,
    failed: bool,
    quitting: bool,
    settings: Settings,
    startup: bool,
    tray: Option<tray::Tray>,
    instance: AppInstance,
    focus: FocusHandle,
    initial: bool,
    background: bool,
}
impl BridgeUi {
    fn new(
        window: &mut Window,
        cx: &mut Context<Self>,
        instance: AppInstance,
        background: bool,
    ) -> Self {
        let loaded = Settings::load();
        let load_error = loaded.as_ref().err().map(|e| format!("{e:#}"));
        let settings = loaded.unwrap_or_default();
        let registered = desktop::startup_value();
        let startup_error = registered.as_ref().err().map(|e| format!("{e:#}"));
        let command = std::env::current_exe()
            .ok()
            .and_then(|p| desktop::startup_command(&p).ok());
        let startup = registered
            .ok()
            .flatten()
            .is_some_and(|v| Some(v) == command);
        let tray = tray::Tray::new();
        let tray_error = tray
            .as_ref()
            .err()
            .map(|e| format!("Tray unavailable: {e:#}"));
        let focus = cx.focus_handle();
        window.focus(&focus);
        cx.on_app_quit(|this, _| {
            this.worker.take();
            this.tray.take();
            async {}
        })
        .detach();
        cx.on_release(|this, _| {
            this.worker.take();
            this.tray.take();
        })
        .detach();
        let entity = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            entity
                .update(cx, |this, cx| {
                    if this.tray.is_some() && !this.quitting {
                        visibility(window, false);
                    } else {
                        this.quit(cx);
                    }
                    false
                })
                .unwrap_or(true)
        });
        let handle = Window::window_handle(window);
        cx.spawn(async move |this, cx| {
            let mut interval = Duration::from_millis(100);
            loop {
                cx.background_executor().timer(interval).await;
                match cx.update_window(handle, |_, window, cx| {
                    this.update(cx, |this, cx| {
                        this.poll(window, cx);
                        Duration::from_millis(
                            if this.settings.diagnostics && this.worker.is_some() {
                                16
                            } else {
                                100
                            },
                        )
                    })
                }) {
                    Ok(Ok(next)) => interval = next,
                    _ => break,
                }
            }
        })
        .detach();
        let mut this = Self {
            worker: None,
            mode: None,
            status: "Ready to connect".into(),
            source: "Raiju V3 Pro → DualSense".into(),
            hz: 0.0,
            fresh_hz: 0.0,
            pc_mode: false,
            statistics: Stats::default(),
            values: VecDeque::with_capacity(4096),
            sticks: None,
            live: None,
            sweep_result: None,
            logs: VecDeque::new(),
            stopping: false,
            failed: false,
            quitting: false,
            settings,
            startup,
            tray: tray.ok(),
            instance,
            focus,
            initial: true,
            background,
        };
        for error in [load_error, startup_error, tray_error]
            .into_iter()
            .flatten()
        {
            this.error(error);
        }
        this
    }
    fn log(&mut self, value: String) {
        self.logs.push_front(value.clone());
        self.logs.truncate(4);
        let _ = std::fs::create_dir_all(desktop::data_root());
        let path = desktop::data_root().join("bridge.log");
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1_000_000) {
            let _ = std::fs::rename(&path, path.with_extension("previous.log"));
        }
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = writeln!(
                f,
                "{} {value}",
                chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
            );
        }
    }
    fn error(&mut self, value: String) {
        self.failed = true;
        self.status = value.clone();
        self.log(value);
    }
    fn start(&mut self, mode: Mode, cx: &mut Context<Self>) {
        if self.worker.is_some() || self.quitting {
            return;
        }
        self.failed = false;
        self.stopping = false;
        self.mode = Some(mode);
        self.hz = 0.0;
        self.live = None;
        self.statistics = Stats::default();
        self.values.clear();
        self.sticks = None;
        self.sweep_result = None;
        self.status = "Connecting…".into();
        self.log(format!("Started {}", mode.name()));
        self.worker = Some(Worker::start(mode, worker::session_directory(mode)));
        self.worker
            .as_ref()
            .unwrap()
            .diagnostics(self.settings.diagnostics);
        cx.notify();
    }
    fn stop(&mut self, cx: &mut Context<Self>) {
        if let Some(worker) = &self.worker {
            worker.stop();
            self.stopping = true;
            self.status = "Stopping…".into();
            cx.notify();
        }
    }
    fn toggle(&mut self, cx: &mut Context<Self>) {
        if self.worker.is_some() {
            self.stop(cx);
        } else {
            self.start(Mode::Bridge, cx);
        }
    }
    fn quit(&mut self, cx: &mut Context<Self>) {
        self.quitting = true;
        if self.worker.is_some() {
            self.stop(cx);
        } else {
            cx.quit();
        }
    }
    fn diagnostics(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.settings.diagnostics = !self.settings.diagnostics;
        if let Err(e) = self.settings.save() {
            self.settings.diagnostics = !self.settings.diagnostics;
            self.error(format!("Could not save settings: {e:#}"));
        }
        self.resize(window);
        if let Some(worker) = &self.worker {
            worker.diagnostics(self.settings.diagnostics);
        }
        if self.settings.diagnostics {
            self.live = None;
        }
        cx.notify();
    }
    fn resize(&self, window: &mut Window) {
        window.resize(if self.settings.diagnostics {
            size(px(900.0), px(940.0))
        } else {
            size(px(580.0), px(530.0))
        });
    }
    fn apply_message(&mut self, message: Message) {
        match message {
            Message::Status(s) => {
                if s.starts_with("Waiting") {
                    self.hz = 0.0;
                    self.live = None;
                }
                if !self.stopping && self.status != s {
                    self.status = s.clone();
                    self.log(s);
                }
            }
            Message::Connected(s) => self.source = format!("Raiju {s} → DualSense"),
            Message::Reports(_, hz, fresh, pc) => {
                self.hz = hz;
                self.fresh_hz = fresh;
                self.pc_mode = pc;
            }
            Message::Sticks(snapshot) => self.sticks = Some(snapshot),
            Message::Diagnostics(snapshot) => self.live = Some(snapshot),
            Message::Button(sample) => {
                if self.values.len() == 4096 {
                    self.values.pop_front();
                }
                self.values.push_back(sample.delay_ms);
                self.logs
                    .push_front(format!("{:.3} ms  {}", sample.delay_ms, sample.action));
                self.logs.truncate(4);
            }
        }
    }
    fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.initial {
            self.initial = false;
            self.resize(window);
            if self.background {
                visibility(window, self.tray.is_none());
            }
            if self.settings.auto_connect {
                self.start(Mode::Bridge, cx);
            }
        }
        if self.instance.take_show_request() {
            visibility(window, true);
            window.activate_window();
        }
        let commands = self.tray.as_ref().map(|t| t.commands()).unwrap_or_default();
        for command in commands {
            match command {
                tray::Command::Show => {
                    visibility(window, true);
                    window.activate_window();
                }
                tray::Command::Toggle => self.toggle(cx),
                tray::Command::Quit => self.quit(cx),
            }
        }
        let messages: Vec<_> = self
            .worker
            .as_ref()
            .map(|w| w.receiver.try_iter().collect())
            .unwrap_or_default();
        let mut dirty = !messages.is_empty();
        for message in messages {
            self.apply_message(message);
        }
        if dirty && !self.values.is_empty() && self.mode == Some(Mode::Buttons) {
            self.statistics = measure::stats(self.values.iter().copied());
        }
        if let Some(outcome) = self.worker.as_mut().and_then(Worker::take_outcome) {
            let mode = self.mode.take();
            self.worker.take();
            self.stopping = false;
            self.hz = 0.0;
            match outcome {
                Outcome::Stopped => {
                    self.failed = false;
                    self.status = "Stopped".into();
                    self.log("Stopped cleanly".into());
                }
                Outcome::Completed(report) => {
                    let report = *report;
                    if let Some(snapshot) = report.diagnostics {
                        self.live = Some(snapshot);
                    }
                    self.failed = false;
                    self.status = if report.directory.is_some() {
                        "Test saved"
                    } else {
                        "Stopped"
                    }
                    .into();
                    if let Some(stats) = report.statistics {
                        self.statistics = stats;
                    }
                    if let Some(sticks) = report.sticks {
                        self.sticks = Some(sticks);
                    }
                    if let Some(sweep) = report.sweep {
                        self.sweep_result = Some(format!(
                            "{} vectors checked · {} changed · max error {} / 255",
                            sweep.tested, sweep.mismatches, sweep.max_error
                        ));
                    }
                    if let Some(path) = report.directory {
                        self.log(format!("Saved {}", path.display()));
                    } else {
                        self.log(format!("{} stopped", mode.unwrap_or(Mode::Bridge).name()));
                    }
                }
                Outcome::Failed(error) => self.error(error),
            }
            dirty = true;
            if self.quitting {
                cx.quit();
                return;
            }
        }
        if let Some(tray) = &self.tray
            && (dirty || self.stopping)
        {
            tray.update(self.worker.is_some(), self.stopping, &self.status);
        }
        if dirty {
            cx.notify();
        }
    }
    fn open_results(&mut self, cx: &mut Context<Self>) {
        let dir = worker::results_root();
        if let Err(e) = std::fs::create_dir_all(&dir).and_then(|_| {
            std::process::Command::new("explorer.exe")
                .arg(dir)
                .spawn()
                .map(|_| ())
        }) {
            self.error(e.to_string());
            cx.notify();
        }
    }
}
fn label(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(11.0))
        .line_height(px(15.0))
        .font_family("Cascadia Mono")
        .text_color(rgb(MUTED))
        .child(text.into())
}
fn button(
    id: &'static str,
    text: &'static str,
    primary: bool,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px_4()
        .py_3()
        .rounded_md()
        .bg(rgb(if primary { ACCENT } else { PANEL }))
        .border_1()
        .border_color(rgb(if primary { ACCENT } else { LINE }))
        .text_color(rgb(if primary { INK } else { TEXT }))
        .font_weight(FontWeight::SEMIBOLD)
        .when(enabled, |s| {
            s.cursor_pointer()
                .hover(|s| s.bg(rgb(if primary { 0xe8ffa0 } else { 0x304143 })))
        })
        .when(!enabled, |s| s.opacity(0.4))
        .child(text)
}
fn switch(id: &'static str, title: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .gap_3()
        .cursor_pointer()
        .py_2()
        .child(
            div()
                .w(px(32.0))
                .h(px(18.0))
                .rounded_full()
                .bg(rgb(if on { ACCENT } else { LINE }))
                .p(px(3.0))
                .flex()
                .when(on, |s| s.justify_end())
                .child(
                    div()
                        .size(px(12.0))
                        .rounded_full()
                        .bg(rgb(if on { INK } else { MUTED })),
                ),
        )
        .child(title)
}
fn metric(title: &'static str, value: Option<f64>) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap_1()
        .child(label(title))
        .child(
            div()
                .text_size(px(22.0))
                .font_family("Cascadia Mono")
                .child(value.map(|v| format!("{v:.3}")).unwrap_or("—".into())),
        )
}
impl Render for BridgeUi {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let running = self.worker.is_some();
        let diag = self.settings.diagnostics;
        div()
            .id("root")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(INK))
            .text_color(rgb(TEXT))
            .font_family("Bahnschrift")
            .text_size(px(14.0))
            .p_6()
            .flex()
            .flex_col()
            .gap_4()
            .when(diag, |s| s.p_5().gap_3())
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if event.keystroke.modifiers.control && key == "d" {
                    this.diagnostics(window, cx);
                } else if event.keystroke.modifiers.control && key == "q" {
                    this.quit(cx);
                } else if key == "space" || key == "enter" {
                    this.toggle(cx);
                }
            }))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(label("RAIJU BRIDGE"))
                    .child(
                        div()
                            .id("hide")
                            .text_size(px(12.0))
                            .text_color(rgb(MUTED))
                            .cursor_pointer()
                            .child(if self.tray.is_some() {
                                "Hide to tray ↘"
                            } else {
                                "Quit"
                            })
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.tray.is_some() {
                                    visibility(window, false);
                                } else {
                                    this.quit(cx);
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .py_2()
                    .when(diag, |s| {
                        s.flex_row().items_center().justify_between().py_0()
                    })
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_3()
                            .child(div().size(px(8.0)).rounded_full().bg(rgb(if self.failed {
                                0xffac8c
                            } else if running {
                                ACCENT
                            } else {
                                MUTED
                            })))
                            .child(
                                div()
                                    .text_size(px(if diag { 22.0 } else { 30.0 }))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(if self.failed {
                                        "Needs attention"
                                    } else if self.stopping {
                                        "Stopping"
                                    } else if self.mode.is_some_and(|m| m != Mode::Bridge) {
                                        "Testing your controller"
                                    } else if self.hz > 0.0 {
                                        "Connected"
                                    } else if running {
                                        "Waiting for controller"
                                    } else {
                                        "Ready when you are"
                                    }),
                            ),
                    )
                    .child(div().text_color(rgb(MUTED)).child(self.status.clone())),
            )
            .child(
                button(
                    "main-toggle",
                    if running {
                        if self.stopping { "Stopping…" } else { "Stop" }
                    } else {
                        "Start bridge"
                    },
                    true,
                    !self.stopping,
                )
                .flex()
                .justify_center()
                .when(diag, |s| s.py_2())
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.stopping {
                        this.toggle(cx);
                    }
                })),
            )
            .when(!diag, |root| {
                root.child(
                    div()
                        .text_size(px(12.0))
                        .text_color(rgb(MUTED))
                        .child("PC or PS5 mode → virtual DualSense · Steam Input off"),
                )
            })
            .child(
                div()
                    .border_y_1()
                    .border_color(rgb(LINE))
                    .py_2()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        switch("diagnostics", "Show diagnostics", diag).on_click(
                            cx.listener(|this, _, window, cx| this.diagnostics(window, cx)),
                        ),
                    )
                    .child(label(if diag && running {
                        if self.pc_mode {
                            format!("{:.0} polls/s · {:.0} new states/s", self.hz, self.fresh_hz)
                        } else {
                            format!("{:.0} reports/s", self.hz)
                        }
                    } else {
                        "".into()
                    })),
            )
            .when(diag, |root| root.child(self.diagnostic_panel(cx)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .when(diag, |s| s.flex_row().gap_5())
                    .child(
                        switch("startup", "Launch at Windows sign-in", self.startup).on_click(
                            cx.listener(|this, _, _, cx| {
                                let enabled = !this.startup;
                                match desktop::set_startup(enabled) {
                                    Ok(()) => {
                                        this.startup = enabled;
                                        this.log(
                                            if enabled {
                                                "Windows startup enabled"
                                            } else {
                                                "Windows startup disabled"
                                            }
                                            .into(),
                                        );
                                    }
                                    Err(e) => {
                                        this.error(format!("Could not change startup: {e:#}"))
                                    }
                                }
                                cx.notify();
                            }),
                        ),
                    )
                    .child(
                        switch(
                            "auto-connect",
                            "Connect automatically when the app opens",
                            self.settings.auto_connect,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.settings.auto_connect = !this.settings.auto_connect;
                            if let Err(e) = this.settings.save() {
                                this.settings.auto_connect = !this.settings.auto_connect;
                                this.error(format!("Could not save settings: {e:#}"));
                            }
                            cx.notify();
                        })),
                    ),
            )
            .child(div().flex_1())
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_3()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .child(if self.tray.is_some() {
                        "Closing this window keeps the bridge in the tray."
                    } else {
                        "Tray unavailable. Closing the window stops the bridge."
                    })
                    .child(
                        div()
                            .id("quit")
                            .cursor_pointer()
                            .child("Quit")
                            .on_click(cx.listener(|this, _, _, cx| this.quit(cx))),
                    ),
            )
    }
}
fn main() {
    if raiju_bridge::touchpad::watchdog_entry() {
        return;
    }
    std::panic::set_hook(Box::new(|info| {
        let _ = std::fs::create_dir_all(desktop::data_root());
        let _ = std::fs::write(
            desktop::data_root().join("crash.log"),
            format!("{}\n{info}", chrono::Local::now()),
        );
    }));
    let instance = match AppInstance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => return,
        Err(e) => {
            let _ = std::fs::create_dir_all(desktop::data_root());
            let _ = std::fs::write(
                desktop::data_root().join("startup-error.log"),
                format!("{e:#}"),
            );
            return;
        }
    };
    let background = std::env::args().any(|a| a == "--background");
    Application::new().run(move |cx: &mut App| {
        let bounds = Bounds::centered(None, size(px(580.0), px(530.0)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(540.0), px(530.0))),
                show: !background,
                focus: !background,
                titlebar: Some(TitlebarOptions {
                    title: Some("Raiju Bridge".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| BridgeUi::new(window, cx, instance, background)),
        );
        if let Err(e) = result {
            let _ = std::fs::create_dir_all(desktop::data_root());
            let _ = std::fs::write(
                desktop::data_root().join("startup-error.log"),
                format!("{e:#}"),
            );
            cx.quit();
        } else if !background {
            cx.activate(true);
        }
    });
}
