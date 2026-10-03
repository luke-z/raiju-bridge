use super::*;
use gpui::{PathBuilder, canvas, point};

pub(super) fn style_titlebar(window: &Window) {
    use windows_sys::Win32::Graphics::Dwm::{
        DWMWA_CAPTION_COLOR, DWMWA_TEXT_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE, DwmSetWindowAttribute,
    };
    if let Ok(handle) = HasWindowHandle::window_handle(window)
        && let RawWindowHandle::Win32(handle) = handle.as_raw()
    {
        let color_ref = |color: u32| (color & 0xff) << 16 | (color & 0xff00) | (color >> 16);
        for (attribute, value) in [
            (DWMWA_USE_IMMERSIVE_DARK_MODE, 1u32),
            (DWMWA_CAPTION_COLOR, color_ref(INK)),
            (DWMWA_TEXT_COLOR, color_ref(MUTED)),
        ] {
            // Cosmetic only: older Windows versions can ignore unsupported attributes.
            unsafe {
                DwmSetWindowAttribute(
                    handle.hwnd.get() as _,
                    attribute as _,
                    (&value as *const u32).cast(),
                    std::mem::size_of::<u32>() as _,
                );
            }
        }
    }
}

pub(super) fn label(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_size(px(12.0))
        .line_height(px(18.0))
        .font_family("Consolas")
        .text_color(rgb(MUTED))
        .child(text.into())
}

pub(super) fn button(
    id: &'static str,
    text: &'static str,
    primary: bool,
    enabled: bool,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .px(px(14.0))
        .py(px(8.0))
        .rounded(px(8.0))
        .border_1()
        .border_color(rgb(if primary { TEXT } else { LINE }))
        .bg(rgb(if primary { TEXT } else { INK }))
        .text_color(rgb(if primary { INK } else { TEXT }))
        .font_weight(FontWeight::MEDIUM)
        .flex_shrink_0()
        .when(enabled, |s| {
            s.cursor_pointer()
                .hover(|s| s.bg(rgb(if primary { 0xffffff } else { PANEL })))
        })
        .when(!enabled, |s| s.opacity(0.4))
        .child(text)
}

pub(super) fn toggle_track(on: bool) -> gpui::Div {
    div()
        .w(px(32.0))
        .h(px(18.0))
        .rounded_full()
        .bg(rgb(if on { TEXT } else { LINE }))
        .p(px(3.0))
        .flex()
        .flex_shrink_0()
        .when(on, |s| s.justify_end())
        .child(
            div()
                .size(px(12.0))
                .rounded_full()
                .bg(rgb(if on { INK } else { MUTED })),
        )
}

fn setting_row(id: &'static str, title: &'static str, on: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .h(px(45.0))
        .px(px(15.0))
        .flex()
        .items_center()
        .justify_between()
        .cursor_pointer()
        .hover(|s| s.bg(rgb(0x141719)))
        .child(
            div()
                .flex()
                .items_center()
                .gap_3()
                .child(title)
                .when(id == "diagnostics", |s| s.child(label("Ctrl D"))),
        )
        .child(toggle_track(on))
}

pub(super) fn metric(title: &'static str, value: Option<f64>, divider: bool) -> gpui::Div {
    div()
        .flex_1()
        .min_w_0()
        .px(px(20.0))
        .py(px(17.0))
        .when(divider, |s| s.border_l_1().border_color(rgb(LINE)))
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .text_color(rgb(MUTED))
                .text_size(px(12.0))
                .child(title),
        )
        .child(
            div()
                .flex()
                .items_baseline()
                .gap_2()
                .child(
                    div()
                        .text_size(px(28.0))
                        .font_family("Consolas")
                        .child(value.map(|v| format!("{v:.3}")).unwrap_or("—".into())),
                )
                .child(label("ms")),
        )
}

fn power_glyph() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds, _, window, _| {
            let p = |x, y| bounds.origin + point(px(x), px(y));
            let mut path = PathBuilder::stroke(px(3.0));
            path.move_to(p(10.0, 10.0));
            path.arc_to(
                point(px(13.0), px(13.0)),
                px(0.0),
                true,
                false,
                p(28.0, 10.0),
            );
            path.move_to(p(19.0, 3.0));
            path.line_to(p(19.0, 17.0));
            if let Ok(path) = path.build() {
                window.paint_path(path, rgb(TEXT));
            }
        },
    )
    .size(px(38.0))
}

impl BridgeUi {
    fn headline(&self) -> &'static str {
        if self.failed {
            "Needs attention"
        } else if self.stopping {
            "Stopping"
        } else if self.mode.is_some_and(|m| m != Mode::Bridge) {
            "Testing controller"
        } else if self.hz > 0.0 {
            "Connected"
        } else if self.worker.is_some() {
            "Connecting"
        } else {
            "Ready to connect"
        }
    }

    fn status_color(&self) -> u32 {
        if self.failed {
            ERROR
        } else if self.hz > 0.0 && !self.stopping {
            ACCENT
        } else {
            MUTED
        }
    }

    fn status_heading(&self, compact: bool) -> gpui::Div {
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(
                div()
                    .size(px(8.0))
                    .rounded_full()
                    .bg(rgb(self.status_color())),
            )
            .child(
                div()
                    .text_size(px(if compact { 27.0 } else { 22.0 }))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.headline()),
            )
    }

    fn power_button(&self, cx: &Context<Self>) -> gpui::Stateful<gpui::Div> {
        div()
            .id("main-toggle")
            .size(px(120.0))
            .rounded_full()
            .border_2()
            .border_color(rgb(self.status_color()))
            .bg(rgb(0x16181a))
            .flex()
            .items_center()
            .justify_center()
            .flex_shrink_0()
            .when(!self.stopping, |s| {
                s.cursor_pointer().hover(|s| s.bg(rgb(0x202326)))
            })
            .when(self.stopping, |s| s.opacity(0.5))
            .child(power_glyph())
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.stopping {
                    this.toggle(cx);
                }
            }))
    }

    fn compact_hero(&self, cx: &Context<Self>) -> gpui::Div {
        div()
            .flex_1()
            .min_h(px(278.0))
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .child(self.power_button(cx))
            .child(div().mt(px(20.0)).child(self.status_heading(true)))
            .child(
                div()
                    .mt_2()
                    .text_color(rgb(MUTED))
                    .child("Raiju V3 Pro → DualSense"),
            )
            .child(
                div()
                    .mt(px(17.0))
                    .flex()
                    .gap_2()
                    .child(
                        self.badge(
                            if self.hz > 0.0 {
                                if self.pc_mode { "PC mode" } else { "PS5 mode" }
                            } else {
                                "PC / PS5 mode"
                            }
                            .into(),
                        ),
                    )
                    .when(self.hz > 0.0, |s| {
                        s.child(self.badge(format!("{} Hz", grouped(self.hz.round() as u64))))
                    }),
            )
            .when(self.failed, |s| {
                s.child(
                    div()
                        .id("error-detail")
                        .mt_3()
                        .max_h(px(70.0))
                        .overflow_y_scroll()
                        .text_size(px(12.0))
                        .text_color(rgb(ERROR))
                        .child(self.status.clone()),
                )
            })
            .when(
                self.worker.is_some() && self.hz == 0.0 && !self.failed,
                |s| {
                    s.child(
                        div()
                            .mt_2()
                            .text_size(px(12.0))
                            .text_color(rgb(MUTED))
                            .child(self.status.clone()),
                    )
                },
            )
    }

    fn badge(&self, text: String) -> gpui::Div {
        label(text)
            .px(px(10.0))
            .py(px(4.0))
            .border_1()
            .border_color(rgb(LINE))
            .rounded_full()
    }

    fn preferences(&self, cx: &Context<Self>) -> gpui::Div {
        div()
            .border_1()
            .border_color(rgb(LINE))
            .rounded(px(10.0))
            .overflow_hidden()
            .flex_shrink_0()
            .child(
                setting_row("diagnostics", "Diagnostics", self.settings.diagnostics)
                    .on_click(cx.listener(|this, _, window, cx| this.diagnostics(window, cx))),
            )
            .child(
                setting_row("startup", "Launch at Windows sign-in", self.startup)
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_startup(cx))),
            )
            .child(
                setting_row(
                    "auto-connect",
                    "Connect when the app opens",
                    self.settings.auto_connect,
                )
                .border_t_1()
                .border_color(rgb(LINE))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.settings.auto_connect = !this.settings.auto_connect;
                    if let Err(e) = this.settings.save() {
                        this.settings.auto_connect = !this.settings.auto_connect;
                        this.error(format!("Could not save settings: {e:#}"));
                    }
                    cx.notify();
                })),
            )
    }

    fn toggle_startup(&mut self, cx: &mut Context<Self>) {
        let enabled = !self.startup;
        match desktop::set_startup(enabled) {
            Ok(()) => {
                self.startup = enabled;
                self.log(
                    if enabled {
                        "Windows startup enabled"
                    } else {
                        "Windows startup disabled"
                    }
                    .into(),
                );
            }
            Err(e) => self.error(format!("Could not change startup: {e:#}")),
        }
        cx.notify();
    }

    fn compact_footer(&self, cx: &Context<Self>) -> gpui::Div {
        div()
            .flex()
            .justify_between()
            .items_center()
            .mt(px(14.0))
            .text_size(px(12.0))
            .text_color(rgb(MUTED))
            .child(if self.tray.is_some() {
                "Closing keeps the bridge running in the tray"
            } else {
                "Closing stops the bridge · tray unavailable"
            })
            .child(
                div()
                    .id("quit")
                    .cursor_pointer()
                    .hover(|s| s.text_color(rgb(TEXT)))
                    .child("Quit")
                    .on_click(cx.listener(|this, _, _, cx| this.quit(cx))),
            )
    }

    fn diagnostic_header(&self, cx: &Context<Self>) -> gpui::Div {
        let running = self.worker.is_some();
        div()
            .flex()
            .items_center()
            .gap_3()
            .h(px(48.0))
            .flex_shrink_0()
            .child(self.status_heading(false))
            .child(
                div()
                    .text_size(px(13.0))
                    .text_color(rgb(MUTED))
                    .child("Raiju V3 Pro → DualSense"),
            )
            .child(div().flex_1())
            .child(label(if self.hz > 0.0 {
                if self.pc_mode {
                    format!(
                        "{} polls/s · {} new/s",
                        grouped(self.hz.round() as u64),
                        grouped(self.fresh_hz.round() as u64)
                    )
                } else {
                    format!("{} reports/s", grouped(self.hz.round() as u64))
                }
            } else {
                String::new()
            }))
            .child(
                button(
                    "main-toggle",
                    if self.stopping {
                        "Stopping…"
                    } else if running {
                        "Stop"
                    } else {
                        "Start"
                    },
                    true,
                    !self.stopping,
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !this.stopping {
                        this.toggle(cx);
                    }
                })),
            )
    }
}

pub(super) fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut result = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            result.push(',');
        }
        result.push(ch);
    }
    result
}

impl Render for BridgeUi {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("root")
            .track_focus(&self.focus)
            .size_full()
            .overflow_y_scroll()
            .bg(rgb(INK))
            .text_color(rgb(TEXT))
            .font_family("Segoe UI")
            .text_size(px(13.0))
            .px(px(28.0))
            .pb(px(22.0))
            .pt(px(12.0))
            .flex()
            .flex_col()
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let key = event.keystroke.key.as_str();
                if event.keystroke.modifiers.control && key == "d" {
                    this.diagnostics(window, cx);
                } else if event.keystroke.modifiers.control && key == "q" {
                    this.quit(cx);
                } else if !event.is_held && !this.stopping && (key == "space" || key == "enter") {
                    this.toggle(cx);
                }
            }))
            .when(!self.settings.diagnostics, |s| {
                s.child(self.compact_hero(cx))
                    .child(self.preferences(cx))
                    .child(self.compact_footer(cx))
            })
            .when(self.settings.diagnostics, |s| {
                s.child(self.diagnostic_header(cx))
                    .when(self.failed, |s| {
                        s.child(
                            div()
                                .text_color(rgb(ERROR))
                                .mb_2()
                                .child(self.status.clone()),
                        )
                    })
                    .child(self.diagnostic_panel(cx))
            })
    }
}
