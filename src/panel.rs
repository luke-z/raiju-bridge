use super::*;
use raiju_bridge::{diagnostics::Snapshot, protocol};

fn keycap(
    text: &'static str,
    mask: u32,
    down: u32,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
) -> gpui::Div {
    let active = down & mask != 0;
    div()
        .absolute()
        .left(px(x))
        .top(px(y))
        .w(px(width))
        .h(px(height))
        .rounded_full()
        .border_1()
        .border_color(rgb(if active { ACCENT } else { LINE }))
        .bg(rgb(if active { ACCENT } else { PANEL }))
        .text_color(rgb(if active { INK } else { TEXT }))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(if width < 32.0 { 17.0 } else { 10.0 }))
        .font_family("Segoe UI Symbol")
        .child(text)
}
fn trigger(text: &'static str, value: u8, x: f32, flashed: bool) -> gpui::Div {
    div()
        .absolute()
        .left(px(x))
        .top_0()
        .w(px(65.0))
        .h(px(17.0))
        .rounded_md()
        .overflow_hidden()
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(if flashed { ACCENT } else { LINE }))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(px(65.0 * value as f32 / 255.0))
                .bg(rgb(ACCENT)),
        )
        .child(
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .text_color(rgb(if value > 127 { INK } else { TEXT }))
                .child(format!("{text}  {value}")),
        )
}
fn stick(x: f32, input: [u8; 2], output: [u8; 2], clicked: bool) -> gpui::Div {
    div()
        .absolute()
        .left(px(x))
        .top(px(120.0))
        .size(px(44.0))
        .rounded_full()
        .bg(rgb(PANEL))
        .border_2()
        .border_color(rgb(if clicked { ACCENT } else { LINE }))
        .child(
            div()
                .absolute()
                .left(px(5.0 + input[0] as f32 / 255.0 * 26.0))
                .top(px(5.0 + input[1] as f32 / 255.0 * 26.0))
                .size(px(6.0))
                .rounded_full()
                .bg(rgb(ACCENT)),
        )
        .child(
            div()
                .absolute()
                .left(px(3.0 + output[0] as f32 / 255.0 * 26.0))
                .top(px(3.0 + output[1] as f32 / 255.0 * 26.0))
                .size(px(10.0))
                .rounded_full()
                .border_1()
                .border_color(rgb(TEXT)),
        )
}
fn touchpad(
    input: &raiju_bridge::touchpad::State,
    output: &raiju_bridge::touchpad::State,
    clicked: bool,
) -> gpui::Div {
    use gpui::{PathBuilder, canvas, point};
    let surface = canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            for (inset, color) in [(0.0, LINE), (1.0, if clicked { ACCENT } else { PANEL })] {
                let mut path = PathBuilder::fill();
                let vertices = [
                    (inset, inset),
                    (110.0 - inset, inset),
                    (100.0 - inset, 51.0 - inset),
                    (10.0 + inset, 51.0 - inset),
                ];
                path.move_to(bounds.origin + point(px(vertices[0].0), px(vertices[0].1)));
                for (x, y) in vertices.into_iter().skip(1) {
                    path.line_to(bounds.origin + point(px(x), px(y)));
                }
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            }
        },
    )
    .absolute()
    .size_full();
    let marker = |c: raiju_bridge::touchpad::Contact, ring: bool| {
        let y = c.y.min(1079) as f32 / 1079.0;
        let x = 6.0 + 10.0 * y + (c.x.min(1919) as f32 / 1919.0) * (98.0 - 20.0 * y);
        let radius = if ring { 5.0 } else { 3.0 };
        div()
            .absolute()
            .left(px(x - radius))
            .top(px(6.0 + y * 39.0 - radius))
            .size(px(radius * 2.0))
            .rounded_full()
            .border_1()
            .border_color(rgb(if ring { TEXT } else { INK }))
            .when(!ring, |v| v.bg(rgb(ACCENT)))
    };
    div()
        .absolute()
        .left(px(167.0))
        .top(px(48.0))
        .w(px(110.0))
        .h(px(51.0))
        .child(surface)
        .child(
            div()
                .relative()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .text_size(px(10.0))
                .text_color(rgb(if clicked { INK } else { MUTED }))
                .child("TOUCH"),
        )
        .children(
            input
                .contacts
                .into_iter()
                .filter(|c| c.active)
                .map(|c| marker(c, false)),
        )
        .children(
            output
                .contacts
                .into_iter()
                .filter(|c| c.active)
                .map(|c| marker(c, true)),
        )
}
fn controller(s: &Snapshot) -> gpui::Div {
    let b = s.highlight_buttons;
    let input = s.sticks.input;
    let output = s.sticks.output;
    div()
        .relative()
        .w(px(440.0))
        .h(px(188.0))
        .flex_shrink_0()
        .child(
            div()
                .absolute()
                .left(px(48.0))
                .top(px(30.0))
                .w(px(344.0))
                .h(px(126.0))
                .rounded(px(46.0))
                .bg(rgb(INK))
                .border_1()
                .border_color(rgb(LINE)),
        )
        .child(
            div()
                .absolute()
                .left(px(44.0))
                .top(px(90.0))
                .w(px(104.0))
                .h(px(90.0))
                .rounded(px(32.0))
                .bg(rgb(INK))
                .border_1()
                .border_color(rgb(LINE)),
        )
        .child(
            div()
                .absolute()
                .left(px(292.0))
                .top(px(90.0))
                .w(px(104.0))
                .h(px(90.0))
                .rounded(px(32.0))
                .bg(rgb(INK))
                .border_1()
                .border_color(rgb(LINE)),
        )
        .child(trigger("L2", s.input_triggers[0], 65.0, b & 0x400 != 0))
        .child(trigger("R2", s.input_triggers[1], 310.0, b & 0x800 != 0))
        .child(keycap("L1", 0x100, b, 65.0, 21.0, 65.0, 18.0))
        .child(keycap("R1", 0x200, b, 310.0, 21.0, 65.0, 18.0))
        .child(keycap("↑", 0x1000000, b, 96.0, 52.0, 25.0, 25.0))
        .child(keycap("←", 0x4000000, b, 74.0, 75.0, 25.0, 25.0))
        .child(keycap("→", 0x8000000, b, 118.0, 75.0, 25.0, 25.0))
        .child(keycap("↓", 0x2000000, b, 96.0, 98.0, 25.0, 25.0))
        .child(keycap("△", 0x80, b, 322.0, 52.0, 28.0, 28.0))
        .child(keycap("□", 0x10, b, 298.0, 77.0, 28.0, 28.0))
        .child(keycap("○", 0x40, b, 346.0, 77.0, 28.0, 28.0))
        .child(keycap("×", 0x20, b, 322.0, 102.0, 28.0, 28.0))
        .child(keycap("·", 0x1000, b, 152.0, 59.0, 17.0, 26.0))
        .child(keycap("≡", 0x2000, b, 273.0, 59.0, 17.0, 26.0))
        .child(touchpad(&s.input_touch, &s.output_touch, b & 0x20000 != 0))
        .child(stick(
            135.0,
            [input[0], input[1]],
            [output[0], output[1]],
            b & 0x4000 != 0,
        ))
        .child(stick(
            265.0,
            [input[2], input[3]],
            [output[2], output[3]],
            b & 0x8000 != 0,
        ))
        .child(keycap("PS", 0x10000, b, 206.0, 119.0, 29.0, 26.0))
        .child(keycap("−", 0x40000, b, 209.0, 154.0, 23.0, 12.0))
}
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
        let idle = self.worker.is_none();
        let s = self.live.as_deref().cloned().unwrap_or_default();
        let latency = if self.mode == Some(Mode::Benchmark)
            || (self.live.is_none() && self.statistics.count > 0)
        {
            &self.statistics
        } else {
            &s.latency
        };
        let held = protocol::button_names(s.highlight_buttons);
        let card = div()
            .bg(rgb(PANEL))
            .rounded_md()
            .p_4()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .child(label(if self.mode == Some(Mode::Bridge) {
                        "LIVE CONTROLLER · 80 ms press flashes"
                    } else {
                        "START BRIDGE TO SEE LIVE INPUT"
                    }))
                    .child(label("Green: input · white ring: output")),
            )
            .child(div().flex().justify_center().child(controller(&s)))
            .child(div().text_size(px(12.0)).child(format!(
                "Pressed / recent: {}",
                if held.is_empty() { "—" } else { &held }
            )))
            .child(label(format!(
                "Sticks: {} exact · {} skipped · {} unmatched | Coverage: {} / {} / {} / {} of 256",
                s.sticks.matched,
                s.sticks.skipped,
                s.sticks.unexpected,
                s.sticks.distinct[0],
                s.sticks.distinct[1],
                s.sticks.distinct[2],
                s.sticks.distinct[3]
            )))
            .child(
                div()
                    .border_t_1()
                    .border_color(rgb(LINE))
                    .pt_2()
                    .flex()
                    .gap_4()
                    .child(metric("DELAY MEDIAN / ms", latency.median_ms))
                    .child(metric("95TH PERCENTILE", latency.p95_ms))
                    .child(metric("MAXIMUM", latency.max_ms)),
            )
            .child(label(format!(
                "{} presses · {} matched edges · {} skipped · {} unmatched | {} queue drops",
                s.button_presses,
                s.button_matches,
                s.button_skipped,
                s.button_unmatched,
                s.source_queue_dropped
            )))
            .when(s.reader_error.is_some(), |v| {
                v.child(div().text_color(rgb(0xffac8c)).child(format!(
                    "Diagnostics paused: {}",
                    s.reader_error.as_deref().unwrap()
                )))
            })
            .when(self.sweep_result.is_some(), |v| {
                v.child(
                    div()
                        .text_color(rgb(ACCENT))
                        .child(self.sweep_result.clone().unwrap()),
                )
            });
        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(card)
            .child(label(
                "Live view runs alongside forwarding. Delay excludes controller, game and display.",
            ))
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
                    .child(div().flex_1())
                    .child(
                        div()
                            .id("results")
                            .cursor_pointer()
                            .text_color(rgb(MUTED))
                            .child("Results ↗")
                            .on_click(cx.listener(|this, _, _, cx| this.open_results(cx))),
                    ),
            )
            .child(label(
                "Synthetic tests take over output. Stop the bridge and close games first.",
            ))
            .child(
                switch(
                    "precise-pc",
                    "Precise 2,000 Hz PC polling · higher CPU · restart bridge to apply",
                    self.settings.precise_pc,
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
            .child(
                div()
                    .h(px(30.0))
                    .overflow_hidden()
                    .children(self.logs.iter().take(1).map(|s| label(s.clone()))),
            )
    }
}
