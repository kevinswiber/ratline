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

#[derive(Clone)]
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

#[derive(Debug)]
pub struct BarRow {
    pub label: String,
    pub value: f64,
    pub total: f64,
    pub state: Option<String>,
}

/// Parse batch rows: `label<DELIM>value<DELIM>total[<DELIM>state]`.
pub fn parse_rows(input: &str, delimiter: char) -> anyhow::Result<Vec<BarRow>> {
    let mut rows = Vec::new();
    for (idx, line) in input.lines().enumerate() {
        if line.is_empty() {
            continue;
        }
        let n = idx + 1;
        let fields: Vec<&str> = line.split(delimiter).collect();
        if fields.len() < 3 {
            anyhow::bail!("line {n}: expected label{delimiter}value{delimiter}total, got {line:?}");
        }
        let value: f64 = fields[1]
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("line {n}: value {:?} is not a number", fields[1]))?;
        let total: f64 = fields[2]
            .trim()
            .parse()
            .map_err(|_| anyhow::anyhow!("line {n}: total {:?} is not a number", fields[2]))?;
        rows.push(BarRow {
            label: fields[0].to_string(),
            value,
            total,
            state: fields.get(3).map(|s| s.to_string()),
        });
    }
    Ok(rows)
}

/// Widest label display width, capped at `max`.
pub fn auto_label_width(rows: &[BarRow], max: u16) -> u16 {
    rows.iter()
        .map(|r| r.label.width().min(usize::from(u16::MAX)) as u16)
        .max()
        .unwrap_or(0)
        .min(max)
}

/// Render every row padded to the spec's shared label column (callers
/// resolve it, via [`auto_label_width`] or an explicit setting).
pub fn render_rows(rows: &[BarRow], spec: &BarSpec<'_>, profile: ColorProfile) -> Vec<String> {
    let label_width = spec.label_width;
    rows.iter()
        .map(|row| {
            let row_spec = BarSpec {
                value: row.value,
                total: row.total,
                label: Some(row.label.as_str()),
                label_width,
                state: row.state.as_deref(),
                fill_style: spec.fill_style.clone(),
                empty_style: spec.empty_style.clone(),
                ..*spec
            };
            render_bar(&row_spec, profile)
        })
        .collect()
}

#[derive(Debug)]
pub struct Threshold {
    pub at: f64,
    pub color: ratatui::style::Color,
}

/// Parse `"33:196,66:214,100:42"` into ascending percentage bands.
pub fn parse_thresholds(s: &str) -> anyhow::Result<Vec<Threshold>> {
    let mut thresholds = Vec::new();
    for part in s.split(',') {
        let (at, color) = part
            .split_once(':')
            .ok_or_else(|| anyhow::anyhow!("threshold {part:?} is not at:color"))?;
        thresholds.push(Threshold {
            at: at
                .trim()
                .parse()
                .map_err(|_| anyhow::anyhow!("threshold {at:?} is not a number"))?,
            color: crate::style_spec::parse_color(color.trim())?,
        });
    }
    thresholds.sort_by(|a, b| a.at.total_cmp(&b.at));
    Ok(thresholds)
}

/// First band whose `at` is >= pct; falls back past the last band.
pub fn color_for(
    pct: f64,
    thresholds: &[Threshold],
    fallback: Option<ratatui::style::Color>,
) -> Option<ratatui::style::Color> {
    thresholds
        .iter()
        .find(|t| pct <= t.at)
        .map(|t| t.color)
        .or_else(|| thresholds.last().map(|t| t.color))
        .or(fallback)
}

/// Moving block for unknown totals: (start, len), period == width.
pub fn indeterminate_span(width: u16, tick: u64) -> (u16, u16) {
    if width == 0 {
        return (0, 0);
    }
    let len = (width / 4).max(1);
    let start = (tick % u64::from(width)) as u16;
    (start, len)
}

/// Render an indeterminate bar: the moving block wraps around the width.
pub fn render_indeterminate(spec: &BarSpec<'_>, tick: u64, profile: ColorProfile) -> String {
    let (start, len) = indeterminate_span(spec.width, tick);
    let mut cells = vec![spec.empty; usize::from(spec.width)];
    for i in 0..len {
        let idx = usize::from((start + i) % spec.width.max(1));
        cells[idx] = spec.fill;
    }
    let mut out = String::new();
    if let Some(label) = spec.label {
        out.push_str(label);
        let pad = usize::from(spec.label_width).saturating_sub(label.width());
        out.push_str(&" ".repeat(pad));
        out.push(' ');
    }
    // Style contiguous runs so fill/empty keep their own colors.
    let mut i = 0usize;
    while i < cells.len() {
        let is_fill = cells[i] == spec.fill;
        let mut j = i;
        while j < cells.len() && (cells[j] == spec.fill) == is_fill {
            j += 1;
        }
        let run: String = cells[i..j].iter().collect();
        let style = if is_fill {
            &spec.fill_style
        } else {
            &spec.empty_style
        };
        out.push_str(&style.render(&run, profile));
        i = j;
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

#[cfg(test)]
mod batch_tests {
    use ratatui::style::Color;

    use super::*;
    use crate::color::ColorProfile;
    use crate::style_spec::StyleSpec;

    #[test]
    fn parse_rows_happy_path() {
        let rows = parse_rows("a\t1\t4\nb\t2\t4\trunning\nc\t3\t4\n", '\t').unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].label, "a");
        assert_eq!(rows[1].state.as_deref(), Some("running"));
        assert_eq!(rows[2].value, 3.0);
        assert_eq!(rows[2].total, 4.0);
    }

    #[test]
    fn parse_rows_missing_field_names_line() {
        let err = parse_rows("a\t1\t4\nbroken\n", '\t')
            .unwrap_err()
            .to_string();
        assert!(err.contains("line 2"), "got: {err}");
    }

    #[test]
    fn parse_rows_non_numeric_is_an_error() {
        let err = parse_rows("a\tx\t4\n", '\t').unwrap_err().to_string();
        assert!(err.contains("line 1"), "got: {err}");
    }

    #[test]
    fn auto_label_width_is_widest_capped() {
        let rows = parse_rows("ab\t1\t2\nlonger-label\t1\t2\n", '\t').unwrap();
        assert_eq!(auto_label_width(&rows, 40), 12);
        assert_eq!(auto_label_width(&rows, 8), 8);
    }

    #[test]
    fn render_rows_pads_equally() {
        let rows = parse_rows("a\t1\t4\nlonger\t3\t4\n", '\t').unwrap();
        let spec = BarSpec {
            value: 0.0,
            total: 0.0,
            width: 8,
            fill: '#',
            empty: '-',
            fill_style: StyleSpec::default(),
            empty_style: StyleSpec::default(),
            label: None,
            label_width: 40,
            annotation: Annotation::Ratio,
            state: None,
        };
        let lines = render_rows(&rows, &spec, ColorProfile::Ascii);
        assert_eq!(lines.len(), 2);
        let bar_at = |s: &str| s.find(['#', '-']).unwrap();
        assert_eq!(bar_at(&lines[0]), bar_at(&lines[1]));
    }

    #[test]
    fn render_rows_trusts_the_spec_label_column() {
        let rows = parse_rows("a\t1\t4\n", '\t').unwrap();
        let spec = BarSpec {
            value: 0.0,
            total: 0.0,
            width: 4,
            fill: '#',
            empty: '-',
            fill_style: StyleSpec::default(),
            empty_style: StyleSpec::default(),
            label: None,
            label_width: 10,
            annotation: Annotation::Ratio,
            state: None,
        };
        let lines = render_rows(&rows, &spec, ColorProfile::Ascii);
        // Label column of 10, then the separator space.
        assert_eq!(lines[0].find(['#', '-']).unwrap(), 11);
    }

    #[test]
    fn thresholds_parse_and_band() {
        let t = parse_thresholds("33:196,66:214,100:42").unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(color_for(0.0, &t, None), Some(Color::Indexed(196)));
        assert_eq!(color_for(33.0, &t, None), Some(Color::Indexed(196)));
        assert_eq!(color_for(50.0, &t, None), Some(Color::Indexed(214)));
        assert_eq!(color_for(100.0, &t, None), Some(Color::Indexed(42)));
        assert_eq!(
            color_for(50.0, &[], Some(Color::Indexed(7))),
            Some(Color::Indexed(7))
        );
    }

    #[test]
    fn bad_thresholds_are_errors() {
        assert!(parse_thresholds("nope").is_err());
        assert!(parse_thresholds("33:notacolor").is_err());
    }

    #[test]
    fn indeterminate_is_periodic() {
        for tick in 0..3 {
            assert_eq!(
                indeterminate_span(32, tick),
                indeterminate_span(32, tick + 32)
            );
        }
        let (_, len) = indeterminate_span(32, 5);
        assert!(len >= 1);
        assert_ne!(indeterminate_span(32, 0), indeterminate_span(32, 1));
    }
}
