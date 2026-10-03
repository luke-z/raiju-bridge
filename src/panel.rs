use super::*;
use crate::{
    controller::controller,
    view::{button, grouped, label, metric, toggle_track},
};
use raiju_bridge::{diagnostics::Snapshot, protocol};

impl BridgeUi {
    pub(super) fn save_diagnostics(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = &self.live else {
            return;
        };
        let directory = worker::results_root().join(format!(
            "diagnostics-{}",
            chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
        ));
        let result = (|| -> anyhow::Result<()> {
            std::fs::create_dir_all(&directory)?;
            let report = serde_json::json!({"recorded_at":chrono::Local::now(),"source":self.source,"snapshot":snapshot,
                "method":"Bounded live observation. Source read completion before translation to matching virtual HID button transitions. Last 4096 matched button delays; cumulative event counters. Stick tuples use the translated 0..255 grid, center 128. PC axes include 16-to-8-bit rounding and Y convention conversion.",
                "scope":"No hardware/game/display latency. Repeated stick tuples are not unique timing tokens. Queue losses, skipped and unmatched events must be considered. An idle capture does not validate full-range accuracy."});
            std::fs::write(
                directory.join("diagnostics.json"),
                serde_json::to_vec_pretty(&report)?,
            )?;
            Ok(())
        })();
        match result {
            Ok(()) => self.log(format!("Snapshot saved · {}", directory.display())),
            Err(e) => self.log(format!("Snapshot could not be saved: {e:#}")),
        }
        cx.notify();
    }

    pub(super) fn diagnostic_panel(&self, cx: &Context<Self>) -> gpui::Div {
        let s = self.live.as_deref().cloned().unwrap_or_default();
        let latency = if self.mode == Some(Mode::Benchmark)
            || (self.live.is_none() && self.statistics.count > 0)
        {
            &self.statistics
        } else {
            &s.latency
        };
        div().flex_1().flex().flex_col().gap(px(16.0)).pt(px(12.0))
            .child(live_input(&s, self.worker.is_some() && self.live.is_some()))
            .child(div().border_1().border_color(rgb(LINE)).rounded(px(10.0)).flex()
                .child(metric("Median delay", latency.median_ms, false))
                .child(metric("95th percentile", latency.p95_ms, true))
                .child(metric("Maximum", latency.max_ms, true)))
            .child(counters(&s))
            .child(div().text_size(px(11.0)).text_color(rgb(MUTED))
                .child("Read → virtual input · excludes controller, game and display · button flashes last 80 ms"))
            .when(s.reader_error.is_some(), |v| v.child(div().text_color(rgb(ERROR))
                .child(format!("Diagnostics paused: {}", s.reader_error.as_deref().unwrap()))))
            .when(self.sweep_result.is_some(), |v| v.child(div().text_color(rgb(TEXT))
                .child(self.sweep_result.clone().unwrap())))
            .child(div().flex_1().min_h(px(8.0)))
            .child(self.diagnostic_tools(&s, cx))
    }

    fn diagnostic_tools(&self, snapshot: &Snapshot, cx: &Context<Self>) -> gpui::Div {
        let idle = self.worker.is_none();
        div()
            .border_t_1()
            .border_color(rgb(LINE))
            .pt(px(16.0))
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(
                        button("save-snapshot", "Save snapshot", false, self.live.is_some())
                            .on_click(cx.listener(|this, _, _, cx| this.save_diagnostics(cx))),
                    )
                    .child(
                        button("sweep", "Stick sweep", false, idle)
                            .on_click(cx.listener(|this, _, _, cx| this.start(Mode::Sweep, cx))),
                    )
                    .child(
                        button("benchmark", "Benchmark", false, idle).on_click(
                            cx.listener(|this, _, _, cx| this.start(Mode::Benchmark, cx)),
                        ),
                    )
                    .child(
                        div()
                            .ml_2()
                            .text_size(px(12.0))
                            .text_color(rgb(MUTED))
                            .child(if idle {
                                "Close games before testing"
                            } else {
                                "Tests need the bridge stopped"
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("results")
                            .cursor_pointer()
                            .text_color(rgb(MUTED))
                            .hover(|s| s.text_color(rgb(TEXT)))
                            .child("Open results ↗")
                            .on_click(cx.listener(|this, _, _, cx| this.open_results(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .child(
                        div()
                            .id("precise-pc")
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .child(toggle_track(self.settings.precise_pc))
                            .child("Precise 2,000 Hz polling")
                            .child(
                                div()
                                    .text_color(rgb(MUTED))
                                    .text_size(px(12.0))
                                    .child("· higher CPU · applies on restart"),
                            )
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.settings.precise_pc = !this.settings.precise_pc;
                                if let Err(e) = this.settings.save() {
                                    this.settings.precise_pc = !this.settings.precise_pc;
                                    this.error(format!("Could not save settings: {e:#}"));
                                }
                                cx.notify();
                            })),
                    )
                    .child(label(snapshot.last_button.clone())),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .text_size(px(11.0))
                    .text_color(rgb(MUTED))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .whitespace_nowrap()
                            .child(self.logs.front().cloned().unwrap_or_default()),
                    )
                    .child(
                        div()
                            .id("compact-view")
                            .ml_3()
                            .flex_shrink_0()
                            .cursor_pointer()
                            .hover(|s| s.text_color(rgb(TEXT)))
                            .child("Compact view  ·  Ctrl D")
                            .on_click(
                                cx.listener(|this, _, window, cx| this.diagnostics(window, cx)),
                            ),
                    ),
            )
    }
}

fn live_input(s: &Snapshot, live: bool) -> gpui::Div {
    let held = protocol::button_names(s.highlight_buttons);
    div()
        .border_1()
        .border_color(rgb(LINE))
        .rounded(px(11.0))
        .p(px(20.0))
        .flex()
        .flex_col()
        .gap_3()
        .child(
            div()
                .flex()
                .justify_between()
                .items_center()
                .child(label(if live {
                    "LIVE INPUT"
                } else {
                    "INPUT PREVIEW · START TO GO LIVE"
                }))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(div().size(px(5.0)).rounded_full().bg(rgb(ACCENT)))
                        .child(label("input"))
                        .child(
                            div()
                                .size(px(6.0))
                                .rounded_full()
                                .border_1()
                                .border_color(rgb(TEXT)),
                        )
                        .child(label("output")),
                ),
        )
        .child(div().flex().justify_center().child(controller(s)))
        .child(
            div()
                .flex()
                .gap(px(24.0))
                .child(trigger_bar("L2", s.input_triggers[0]))
                .child(trigger_bar("R2", s.input_triggers[1])),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .min_h(px(22.0))
                .child(div().text_color(rgb(MUTED)).child("Pressed"))
                .when(held.is_empty(), |v| {
                    v.child(div().text_color(rgb(MUTED)).child("—"))
                })
                .children(held.split(" + ").filter(|s| !s.is_empty()).map(|name| {
                    div()
                        .px_2()
                        .py(px(2.0))
                        .rounded(px(5.0))
                        .bg(rgb(PANEL))
                        .child(name.to_string())
                })),
        )
}

fn trigger_bar(name: &'static str, value: u8) -> gpui::Div {
    div()
        .flex_1()
        .flex()
        .flex_col()
        .gap_2()
        .child(
            div()
                .flex()
                .justify_between()
                .child(label(name))
                .child(label(value.to_string()).text_color(rgb(TEXT))),
        )
        .child(
            div()
                .h(px(4.0))
                .rounded_full()
                .bg(rgb(PANEL))
                .overflow_hidden()
                .child(
                    div()
                        .h_full()
                        .w(gpui::relative(value as f32 / 255.0))
                        .rounded_full()
                        .bg(rgb(TEXT)),
                ),
        )
}

fn counter_row(title: &'static str, value: String) -> gpui::Div {
    div()
        .flex()
        .justify_between()
        .items_center()
        .gap_3()
        .text_size(px(13.0))
        .child(div().text_color(rgb(MUTED)).child(title))
        .child(label(value).text_color(rgb(TEXT)))
}

fn counters(s: &Snapshot) -> gpui::Div {
    div()
        .flex()
        .gap(px(36.0))
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_2()
                .child(label("STICKS"))
                .child(counter_row(
                    "Exact / skipped / unmatched",
                    format!(
                        "{} · {} · {}",
                        grouped(s.sticks.matched),
                        grouped(s.sticks.skipped),
                        grouped(s.sticks.unexpected)
                    ),
                ))
                .child(counter_row(
                    "Coverage of 256",
                    s.sticks
                        .distinct
                        .iter()
                        .map(usize::to_string)
                        .collect::<Vec<_>>()
                        .join(" · "),
                )),
        )
        .child(
            div()
                .flex_1()
                .flex()
                .flex_col()
                .gap_2()
                .child(label("BUTTONS"))
                .child(counter_row(
                    "Presses / matched edges",
                    format!(
                        "{} · {}",
                        grouped(s.button_presses),
                        grouped(s.button_matches)
                    ),
                ))
                .child(counter_row(
                    "Skipped / unmatched / drops",
                    format!(
                        "{} · {} · {}",
                        grouped(s.button_skipped),
                        grouped(s.button_unmatched),
                        grouped(s.source_queue_dropped)
                    ),
                )),
        )
}
