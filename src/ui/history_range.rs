use super::*;

const FONT_SIZE: f32 = 11.;
const LINE_HEIGHT: f32 = 16.;
const RULE_WIDTH: f32 = 7.;
const RULE_THICKNESS: f32 = 0.85;
const RULE_OPTICAL_LIFT: f32 = 0.25;

fn separator_mask() -> HighlightStyle {
    HighlightStyle {
        // Highlight colors are blended over the base color in GPUI: a
        // transparent highlight leaves the original glyph fully visible.
        // Fade its ink to zero while preserving shaping and text advance.
        fade_out: Some(1.),
        ..Default::default()
    }
}

// Keep the semantic en dash, with narrow nonbreaking spaces for readable gaps.
// Only its ink is replaced; text layout and accessible text retain the range.
pub(super) fn display_text(text: &str) -> String {
    text.replace('–', "\u{202f}\u{202f}–\u{202f}\u{202f}")
}

pub(super) fn align_rule_to_pixels(bounds: Bounds<Pixels>, scale: f32) -> Bounds<Pixels> {
    let width = (bounds.size.width * scale).round().max(px(1.));
    let height = (bounds.size.height * scale).round().max(px(1.));
    let center = bounds.center();
    // Snap the absolute screen position, after all parent offsets. Quantize
    // dimensions first so the stroke cannot alternate between device rows.
    Bounds::new(
        point(
            (center.x * scale - width / 2.).round() / scale,
            (center.y * scale - height / 2.).round() / scale,
        ),
        size(width / scale, height / scale),
    )
}

pub(super) fn separator_rules(
    text: &str,
    width: Option<Pixels>,
    style: &TextStyle,
    window: &Window,
) -> Vec<Bounds<Pixels>> {
    let system = window.text_system();
    let zero = system.shape_line("0".into(), px(FONT_SIZE), &[style.to_run(1)], None);
    let ink = zero
        .font_id_for_index(0)
        .and_then(|font| system.typographic_bounds(font, px(FONT_SIZE), '0').ok());
    let Ok(lines) = system.shape_text(
        text.to_owned().into(),
        px(FONT_SIZE),
        &[style.to_run(text.len())],
        width,
        None,
    ) else {
        return vec![];
    };
    let mut top = px(0.);
    let mut rules = Vec::new();
    for line in lines {
        let baseline = (px(LINE_HEIGHT) - line.ascent() - line.descent()) / 2. + line.ascent();
        let ink_center = ink.map_or((line.ascent() - line.descent()) / 2., |b| {
            b.origin.y + b.size.height / 2.
        });
        for (index, _) in line.text.match_indices('–') {
            let Some(start) = line.position_for_index(index, px(LINE_HEIGHT)) else {
                continue;
            };
            let advance = line.unwrapped_layout.x_for_index(index + '–'.len_utf8())
                - line.unwrapped_layout.x_for_index(index);
            // Keep the rule light; the painter aligns the final thin stroke
            // to whole device pixels instead of blending fractional edges.
            let thickness = px(RULE_THICKNESS);
            rules.push(Bounds::new(
                point(
                    start.x + (advance - px(RULE_WIDTH)) / 2.,
                    top + start.y + baseline - ink_center - px(RULE_OPTICAL_LIFT) - thickness / 2.,
                ),
                size(px(RULE_WIDTH), thickness),
            ));
        }
        top += line.size(px(LINE_HEIGHT)).height;
    }
    rules
}

pub(super) fn label(text: String, color: Option<u32>, weight: FontWeight, wrap: bool) -> Div {
    if !text.contains('–') {
        return div()
            .text_size(px(FONT_SIZE))
            .line_height(px(LINE_HEIGHT))
            .font_weight(weight)
            .when_some(color, |d, color| d.text_color(rgb(color)))
            .child(text);
    }
    let text = display_text(&text);
    let hidden = text
        .match_indices('–')
        .map(|(index, _)| (index..index + '–'.len_utf8(), separator_mask()))
        .collect::<Vec<_>>();
    let painted = text.clone();
    div()
        .relative()
        .font_family("Microsoft YaHei UI")
        .text_size(px(FONT_SIZE))
        .line_height(px(LINE_HEIGHT))
        .font_weight(weight)
        .when_some(color, |d, color| d.text_color(rgb(color)))
        .when(wrap, |d| d.whitespace_normal())
        .when(!wrap, |d| d.whitespace_nowrap().overflow_hidden())
        .child(StyledText::new(text).with_highlights(hidden))
        .child(
            canvas(
                move |bounds, window, _| {
                    let style = window.text_style();
                    let rules = separator_rules(
                        &painted,
                        wrap.then_some(bounds.size.width),
                        &style,
                        window,
                    );
                    (rules, style.color)
                },
                |bounds, (rules, color), window, _| {
                    window.with_content_mask(Some(ContentMask { bounds }), |window| {
                        for rule in rules {
                            let aligned = align_rule_to_pixels(
                                Bounds::new(bounds.origin + rule.origin, rule.size),
                                window.scale_factor(),
                            );
                            window.paint_quad(fill(aligned, color));
                        }
                    });
                },
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full(),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[::core::prelude::v1::test]
    fn replacing_a_separator_removes_the_original_glyph_ink() {
        for color in [0x202020, 0x606060, 0xD8D8D8, 0x963A67] {
            let style = TextStyle {
                color: rgb(color).into(),
                ..Default::default()
            };
            let masked = style.clone().highlight(separator_mask());
            assert_eq!(
                masked.color.a, 0.,
                "original separator still paints over the custom rule for {color:06x}"
            );
            assert_eq!(masked.font(), style.font(), "mask changed glyph layout");
        }
    }

    #[::core::prelude::v1::test]
    fn rules_keep_integer_device_edges_and_consistent_size_at_fractional_origins() {
        for scale in [1., 1.25, 1.5, 1.75, 2., 2.5, 3.] {
            for offset in [-0.7, 0., 0.1, 0.25, 0.333, 0.5, 0.75] {
                let original = Bounds::new(
                    point(px(595.9 + offset), px(228.19 + offset)),
                    size(px(RULE_WIDTH), px(RULE_THICKNESS)),
                );
                let aligned = align_rule_to_pixels(original, scale);
                for edge in [
                    aligned.left(),
                    aligned.right(),
                    aligned.top(),
                    aligned.bottom(),
                ] {
                    let device = f32::from(edge) * scale;
                    assert!(
                        (device - device.round()).abs() < 0.001,
                        "fractional device edge at scale {scale}: {aligned:?}"
                    );
                }
                assert!((aligned.center().x - original.center().x).abs() * scale <= px(0.501));
                assert!((aligned.center().y - original.center().y).abs() * scale <= px(0.501));
                assert!(
                    (f32::from(aligned.size.width) * scale - (RULE_WIDTH * scale).round()).abs()
                        < 0.001
                );
                assert!(
                    (f32::from(aligned.size.height) * scale
                        - (RULE_THICKNESS * scale).round().max(1.))
                    .abs()
                        < 0.001
                );
            }
        }
    }
    #[::core::prelude::v1::test]
    fn range_gaps_do_not_add_break_opportunities_or_change_single_minutes() {
        assert_eq!(
            display_text("13:00–14:00"),
            "13:00\u{202f}\u{202f}–\u{202f}\u{202f}14:00"
        );
        assert_eq!(
            display_text("2026年 · 10月19–25日"),
            "2026年 · 10月19\u{202f}\u{202f}–\u{202f}\u{202f}25日"
        );
        assert_eq!(display_text("1:19"), "1:19");
    }
}
