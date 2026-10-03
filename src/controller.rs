use super::*;
use gpui::{PathBuilder, canvas, point};
use raiju_bridge::{diagnostics::Snapshot, touchpad};

fn keycap(text: &'static str, mask: u32, down: u32, rect: [f32; 4], round: bool) -> gpui::Div {
    let active = down & mask != 0;
    div()
        .absolute()
        .left(px(rect[0]))
        .top(px(rect[1]))
        .w(px(rect[2]))
        .h(px(rect[3]))
        .rounded(px(if round { rect[2] / 2.0 } else { 4.0 }))
        .border_1()
        .border_color(rgb(if active { TEXT } else { 0x34393f }))
        .bg(rgb(if active { TEXT } else { PANEL }))
        .text_color(rgb(if active { INK } else { MUTED }))
        .flex()
        .items_center()
        .justify_center()
        .text_size(px(22.0))
        .font_family("Segoe UI Symbol")
        .child(text)
}

fn stick(x: f32, input: [u8; 2], output: [u8; 2], clicked: bool) -> gpui::Div {
    let marker = |axes: [u8; 2], ring: bool| {
        let radius = if ring { 6.0 } else { 3.5 };
        div()
            .absolute()
            .left(px(8.0 + axes[0] as f32 / 255.0 * 40.0 - radius))
            .top(px(8.0 + axes[1] as f32 / 255.0 * 40.0 - radius))
            .size(px(radius * 2.0))
            .rounded_full()
            .when(ring, |v| v.border_1().border_color(rgb(TEXT)))
            .when(!ring, |v| v.bg(rgb(ACCENT)))
    };
    div()
        .absolute()
        .left(px(x))
        .top(px(145.0))
        .size(px(56.0))
        .rounded_full()
        .bg(rgb(PANEL))
        .border_1()
        .border_color(rgb(if clicked { TEXT } else { 0x34393f }))
        .child(marker(input, false))
        .child(marker(output, true))
}

fn touch_surface(clicked: bool) -> impl IntoElement {
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| {
            for (inset, color) in [
                (0.0, 0x34393f),
                (1.5, if clicked { 0x303538 } else { 0x191c1f }),
            ] {
                let p = |x, y| bounds.origin + point(px(x), px(y));
                let mut path = PathBuilder::fill();
                path.move_to(p(8.0, inset));
                path.line_to(p(132.0, inset));
                path.curve_to(p(140.0 - inset, 8.0), p(140.0 - inset, inset));
                path.line_to(p(130.0 - inset, 62.0));
                path.curve_to(p(122.0, 70.0 - inset), p(130.0 - inset, 70.0 - inset));
                path.line_to(p(18.0, 70.0 - inset));
                path.curve_to(p(10.0 + inset, 62.0), p(10.0 + inset, 70.0 - inset));
                path.line_to(p(inset, 8.0));
                path.curve_to(p(8.0, inset), p(inset, inset));
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            }
        },
    )
    .absolute()
    .size_full()
}

fn touchpad(input: &touchpad::State, output: &touchpad::State, clicked: bool) -> gpui::Div {
    let marker = |c: touchpad::Contact, ring: bool| {
        let y = c.y.min(1079) as f32 / 1079.0;
        // Project the logical rectangle onto the Raiju's wider top edge.
        let x = 8.0 + 10.0 * y + c.x.min(1919) as f32 / 1919.0 * (124.0 - 20.0 * y);
        let radius = if ring { 7.0 } else { 4.0 };
        div()
            .absolute()
            .left(px(x - radius))
            .top(px(8.0 + y * 54.0 - radius))
            .size(px(radius * 2.0))
            .rounded_full()
            .when(ring, |v| v.border_1().border_color(rgb(TEXT)))
            .when(!ring, |v| v.bg(rgb(ACCENT)))
    };
    div()
        .absolute()
        .left(px(180.0))
        .top(px(43.0))
        .w(px(140.0))
        .h(px(70.0))
        .child(touch_surface(clicked))
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

fn shell() -> impl IntoElement {
    canvas(
        |_, _, _| (),
        |bounds, _, window, _| {
            for (mut path, color) in [
                (PathBuilder::fill(), 0x16181b),
                (PathBuilder::stroke(px(1.5)), LINE),
            ] {
                let p = |x, y| bounds.origin + point(px(x), px(y));
                path.move_to(p(137.0, 32.0));
                path.line_to(p(363.0, 32.0));
                path.cubic_bezier_to(p(452.0, 91.0), p(414.0, 32.0), p(445.0, 55.0));
                path.line_to(p(482.0, 214.0));
                path.cubic_bezier_to(p(448.0, 278.0), p(490.0, 258.0), p(470.0, 278.0));
                path.cubic_bezier_to(p(399.0, 250.0), p(425.0, 278.0), p(411.0, 267.0));
                path.line_to(p(361.0, 207.0));
                path.line_to(p(139.0, 207.0));
                path.line_to(p(101.0, 250.0));
                path.cubic_bezier_to(p(52.0, 278.0), p(89.0, 267.0), p(75.0, 278.0));
                path.cubic_bezier_to(p(18.0, 214.0), p(30.0, 278.0), p(10.0, 258.0));
                path.line_to(p(48.0, 91.0));
                path.cubic_bezier_to(p(137.0, 32.0), p(55.0, 55.0), p(86.0, 32.0));
                path.close();
                if let Ok(path) = path.build() {
                    window.paint_path(path, rgb(color));
                }
            }
        },
    )
    .absolute()
    .size_full()
}

pub(super) fn controller(s: &Snapshot) -> gpui::Div {
    let b = s.highlight_buttons;
    let input = s.sticks.input;
    let output = s.sticks.output;
    div()
        .relative()
        .w(px(500.0))
        .h(px(290.0))
        .flex_shrink_0()
        .child(shell())
        .child(keycap("", 0x100, b, [75.0, 8.0, 90.0, 16.0], true))
        .child(keycap("", 0x200, b, [335.0, 8.0, 90.0, 16.0], true))
        .child(keycap("", 0x1000000, b, [101.0, 74.0, 18.0, 22.0], false))
        .child(keycap("", 0x4000000, b, [77.0, 98.0, 22.0, 18.0], false))
        .child(keycap("", 0x8000000, b, [121.0, 98.0, 22.0, 18.0], false))
        .child(keycap("", 0x2000000, b, [101.0, 118.0, 18.0, 22.0], false))
        .child(keycap("△", 0x80, b, [375.0, 68.0, 25.0, 25.0], true))
        .child(keycap("□", 0x10, b, [349.0, 94.0, 25.0, 25.0], true))
        .child(keycap("○", 0x40, b, [401.0, 94.0, 25.0, 25.0], true))
        .child(keycap("×", 0x20, b, [375.0, 120.0, 25.0, 25.0], true))
        .child(keycap("", 0x1000, b, [153.0, 52.0, 9.0, 20.0], false))
        .child(keycap("", 0x2000, b, [338.0, 52.0, 9.0, 20.0], false))
        .child(touchpad(&s.input_touch, &s.output_touch, b & 0x20000 != 0))
        .child(stick(
            155.0,
            [input[0], input[1]],
            [output[0], output[1]],
            b & 0x4000 != 0,
        ))
        .child(stick(
            289.0,
            [input[2], input[3]],
            [output[2], output[3]],
            b & 0x8000 != 0,
        ))
        .child(keycap("", 0x10000, b, [240.0, 153.0, 20.0, 20.0], true))
        .child(keycap("", 0x40000, b, [240.0, 188.0, 20.0, 8.0], false))
}
