use unicode_width::UnicodeWidthStr;

use crate::color::ColorProfile;
use crate::style_spec::StyleSpec;

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Annotation {
    None,
    Percent,
    Ratio,
    Both,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum BarPreset {
    Blocks,
    Shade,
    Ascii,
    Line,
    Dots,
}

impl BarPreset {
    pub fn chars(self) -> (char, char) {
        match self {
            BarPreset::Blocks => ('\u{2588}', '\u{2591}'),
            BarPreset::Shade => ('\u{2593}', '\u{2591}'),
            BarPreset::Ascii => ('#', '-'),
            BarPreset::Line => ('\u{2501}', '\u{2500}'),
            BarPreset::Dots => ('\u{28ff}', '\u{28c0}'),
        }
    }
}

pub struct BarSpec<'a> {
    pub value: f64,
    pub total: f64,
    pub width: u16,
    pub fill: char,
    pub empty: char,
    pub fill_style: StyleSpec,
    pub empty_style: StyleSpec,
    pub label: Option<&'a str>,
    pub label_width: u16,
    pub annotation: Annotation,
    pub state: Option<&'a str>,
}

pub fn filled_cells(value: f64, total: f64, width: u16) -> u16 {
    if total <= 0.0 || width == 0 {
        return 0;
    }
    let clamped = value.clamp(0.0, total);
    ((clamped * f64::from(width)) / total).floor() as u16
}

pub fn render_bar(spec: &BarSpec<'_>, profile: ColorProfile) -> String {
    let filled = filled_cells(spec.value, spec.total, spec.width);
    let empty = spec.width - filled;
    let mut out = String::new();

    if let Some(label) = spec.label {
        out.push_str(label);
        let pad = usize::from(spec.label_width).saturating_sub(label.width());
        out.push_str(&" ".repeat(pad));
        out.push(' ');
    }

    if filled > 0 {
        let text: String = std::iter::repeat_n(spec.fill, usize::from(filled)).collect();
        out.push_str(&spec.fill_style.render(&text, profile));
    }
    if empty > 0 {
        let text: String = std::iter::repeat_n(spec.empty, usize::from(empty)).collect();
        out.push_str(&spec.empty_style.render(&text, profile));
    }

    let show_ratio = matches!(spec.annotation, Annotation::Ratio | Annotation::Both);
    let show_pct = matches!(spec.annotation, Annotation::Percent | Annotation::Both);
    if show_ratio {
        let value_i = spec.value.round() as i64;
        let total_i = spec.total.round() as i64;
        let w = total_i.to_string().len();
        out.push_str(&format!("  {value_i:>w$}/{total_i:<w$}"));
    }
    if show_pct {
        let pct = if spec.total > 0.0 {
            spec.value.clamp(0.0, spec.total) * 100.0 / spec.total
        } else {
            0.0
        };
        // Truncate (not round) to one decimal, matching fish math -s1.
        let pct = (pct * 10.0).floor() / 10.0;
        out.push_str(&format!(
            "{}{pct:>5.1}%",
            if show_ratio { " " } else { "  " }
        ));
    }
    if let Some(state) = spec.state {
        out.push_str("  ");
        out.push_str(state);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::ColorProfile;
    use crate::style_spec::StyleSpec;

    #[test]
    fn filled_cells_edges() {
        assert_eq!(filled_cells(0.0, 100.0, 32), 0);
        assert_eq!(filled_cells(100.0, 100.0, 32), 32);
        assert_eq!(filled_cells(150.0, 100.0, 32), 32);
        assert_eq!(filled_cells(-5.0, 100.0, 32), 0);
        assert_eq!(filled_cells(50.0, 100.0, 32), 16);
        assert_eq!(filled_cells(1242.0, 1288.0, 32), 30);
    }

    #[test]
    fn zero_total_and_zero_width_do_not_panic() {
        assert_eq!(filled_cells(5.0, 0.0, 32), 0);
        assert_eq!(filled_cells(5.0, 100.0, 0), 0);
    }

    fn fish_spec<'a>() -> BarSpec<'a> {
        BarSpec {
            value: 1242.0,
            total: 1288.0,
            width: 32,
            fill: '█',
            empty: '░',
            fill_style: StyleSpec::default(),
            empty_style: StyleSpec::default(),
            label: Some("L100 release recovery"),
            label_width: 34,
            annotation: Annotation::Both,
            state: Some("running"),
        }
    }

    #[test]
    fn render_matches_fish_layout_exactly() {
        let expected = format!(
            "{}{} {}{}  1242/1288  96.4%  running",
            "L100 release recovery",
            " ".repeat(13),
            "█".repeat(30),
            "░".repeat(2),
        );
        assert_eq!(render_bar(&fish_spec(), ColorProfile::Ascii), expected);
    }

    #[test]
    fn render_colors_fill_and_empty_separately() {
        let mut spec = fish_spec();
        spec.fill_style = StyleSpec {
            foreground: Some(ratatui::style::Color::Indexed(212)),
            ..StyleSpec::default()
        };
        spec.empty_style = StyleSpec {
            foreground: Some(ratatui::style::Color::Indexed(240)),
            ..StyleSpec::default()
        };
        let out = render_bar(&spec, ColorProfile::Ansi256);
        assert!(
            out.contains("\x1b[38;5;212m"),
            "fill color missing: {out:?}"
        );
        assert!(
            out.contains("\x1b[38;5;240m"),
            "empty color missing: {out:?}"
        );
    }

    #[test]
    fn wide_label_pads_by_display_width() {
        let mut spec = fish_spec();
        spec.label = Some("日本");
        spec.label_width = 6;
        let out = render_bar(&spec, ColorProfile::Ascii);
        assert!(out.starts_with("日本   █"), "got: {out:?}");
    }

    #[test]
    fn no_label_skips_the_column() {
        let mut spec = fish_spec();
        spec.label = None;
        let out = render_bar(&spec, ColorProfile::Ascii);
        assert!(out.starts_with('█'), "got: {out:?}");
    }

    #[test]
    fn annotation_variants() {
        let mut spec = fish_spec();
        spec.state = None;
        spec.annotation = Annotation::None;
        let out = render_bar(&spec, ColorProfile::Ascii);
        assert!(!out.contains('%') && !out.contains("1242/"), "got: {out:?}");
        spec.annotation = Annotation::Percent;
        let out = render_bar(&spec, ColorProfile::Ascii);
        assert!(
            out.contains("96.4%") && !out.contains("1242/"),
            "got: {out:?}"
        );
        spec.annotation = Annotation::Ratio;
        let out = render_bar(&spec, ColorProfile::Ascii);
        assert!(
            out.contains("1242/1288") && !out.contains('%'),
            "got: {out:?}"
        );
    }

    #[test]
    fn presets_have_expected_glyphs() {
        assert_eq!(BarPreset::Blocks.chars(), ('█', '░'));
        assert_eq!(BarPreset::Shade.chars(), ('▓', '░'));
        assert_eq!(BarPreset::Ascii.chars(), ('#', '-'));
        assert_eq!(BarPreset::Line.chars(), ('━', '─'));
        assert_eq!(BarPreset::Dots.chars(), ('⣿', '⣀'));
    }
}
