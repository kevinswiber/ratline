use ratatui::symbols::border::Set;

use crate::color::ColorProfile;
use crate::core::measure::{Align, ELLIPSIS, display_width, pad_display, truncate_display};
use crate::style_spec::StyleSpec;

#[derive(Copy, Clone, PartialEq, Eq, Debug, Default, clap::ValueEnum)]
pub enum BorderPreset {
    #[default]
    None,
    Rounded,
    Normal,
    Thick,
    Double,
    Ascii,
}

/// The dumb-terminal border, for terminals without box-drawing glyphs.
const ASCII: Set<'static> = Set {
    top_left: "+",
    top_right: "+",
    bottom_left: "+",
    bottom_right: "+",
    vertical_left: "|",
    vertical_right: "|",
    horizontal_top: "-",
    horizontal_bottom: "-",
};

impl BorderPreset {
    /// The glyph set, or `None` for [`BorderPreset::None`].
    pub fn set(self) -> Option<Set<'static>> {
        use ratatui::symbols::border;
        match self {
            BorderPreset::None => None,
            BorderPreset::Rounded => Some(border::ROUNDED),
            BorderPreset::Normal => Some(border::PLAIN),
            BorderPreset::Thick => Some(border::THICK),
            BorderPreset::Double => Some(border::DOUBLE),
            BorderPreset::Ascii => Some(ASCII),
        }
    }
}

/// Space on the four sides, in cells.
#[derive(Copy, Clone, PartialEq, Eq, Debug, Default)]
pub struct Sides {
    pub top: usize,
    pub right: usize,
    pub bottom: usize,
    pub left: usize,
}

/// Parse CSS shorthand: "1" | "1 2" | "1 2 3" | "1 2 3 4" (commas accepted).
pub fn parse_sides(s: &str) -> anyhow::Result<Sides> {
    let values: Vec<usize> = s
        .split([' ', ','])
        .filter(|part| !part.is_empty())
        .map(|part| {
            part.parse()
                .map_err(|_| anyhow::anyhow!("invalid side {part:?}"))
        })
        .collect::<Result<_, _>>()?;
    match values[..] {
        [all] => Ok(Sides {
            top: all,
            right: all,
            bottom: all,
            left: all,
        }),
        [vertical, horizontal] => Ok(Sides {
            top: vertical,
            right: horizontal,
            bottom: vertical,
            left: horizontal,
        }),
        [top, horizontal, bottom] => Ok(Sides {
            top,
            right: horizontal,
            bottom,
            left: horizontal,
        }),
        [top, right, bottom, left] => Ok(Sides {
            top,
            right,
            bottom,
            left,
        }),
        _ => Err(anyhow::anyhow!(
            "expected 1 to 4 values (top right bottom left), got {s:?}"
        )),
    }
}

pub struct BoxSpec<'a> {
    pub content_style: StyleSpec,
    pub width: Option<usize>,
    pub align: Align,
    pub padding: Sides,
    pub border: BorderPreset,
    pub border_style: StyleSpec,
    pub title: Option<&'a str>,
    pub margin: Sides,
    pub ellipsis: &'a str,
}

impl Default for BoxSpec<'_> {
    fn default() -> Self {
        BoxSpec {
            content_style: StyleSpec::default(),
            width: None,
            align: Align::default(),
            padding: Sides::default(),
            border: BorderPreset::default(),
            border_style: StyleSpec::default(),
            title: None,
            margin: Sides::default(),
            ellipsis: ELLIPSIS,
        }
    }
}

/// Style, align, pad, border, and margin a block of content lines.
pub fn render_box(lines: &[String], spec: &BoxSpec<'_>, profile: ColorProfile) -> Vec<String> {
    let inner = spec
        .width
        .unwrap_or_else(|| lines.iter().map(|l| display_width(l)).max().unwrap_or(0));
    // The bare path adds no trailing whitespace; anything that closes the
    // right edge (border, right padding, pinned width) squares it up.
    let need_right_pad =
        spec.width.is_some() || spec.border.set().is_some() || spec.padding.right > 0;

    let blank = " ".repeat(inner + spec.padding.left + spec.padding.right);
    let mut interior: Vec<String> = Vec::new();
    for _ in 0..spec.padding.top {
        interior.push(blank.clone());
    }
    for line in lines {
        let cut = truncate_display(line, inner, spec.ellipsis);
        let aligned = if need_right_pad {
            pad_display(&cut, inner, spec.align)
        } else {
            align_without_trailing(&cut, inner, spec.align)
        };
        interior.push(format!(
            "{}{}{}",
            " ".repeat(spec.padding.left),
            aligned,
            " ".repeat(spec.padding.right)
        ));
    }
    for _ in 0..spec.padding.bottom {
        interior.push(blank.clone());
    }

    // Content styling covers the padding too, so backgrounds fill the box.
    let interior: Vec<String> = interior
        .iter()
        .map(|line| spec.content_style.render(line, profile))
        .collect();

    let boxed = match spec.border.set() {
        None => interior,
        Some(set) => {
            let run = inner + spec.padding.left + spec.padding.right;
            let mut boxed = Vec::with_capacity(interior.len() + 2);
            boxed.push(top_border(&set, run, spec, profile));
            for line in interior {
                boxed.push(format!(
                    "{}{line}{}",
                    spec.border_style.render(set.vertical_left, profile),
                    spec.border_style.render(set.vertical_right, profile)
                ));
            }
            boxed.push(spec.border_style.render(
                &format!(
                    "{}{}{}",
                    set.bottom_left,
                    set.horizontal_bottom.repeat(run),
                    set.bottom_right
                ),
                profile,
            ));
            boxed
        }
    };

    let mut out = Vec::new();
    for _ in 0..spec.margin.top {
        out.push(String::new());
    }
    let left = " ".repeat(spec.margin.left);
    for line in boxed {
        if line.is_empty() {
            out.push(line);
        } else {
            out.push(format!("{left}{line}"));
        }
    }
    for _ in 0..spec.margin.bottom {
        out.push(String::new());
    }
    // margin.right is accepted so the shorthand parses symmetrically, but
    // line output has no right edge to move.
    out
}

/// The top border, with the title spliced in when the box is wide enough.
/// The title text is inserted verbatim — the border style never wraps it, so
/// a pre-styled title keeps its own look.
fn top_border(set: &Set<'_>, run: usize, spec: &BoxSpec<'_>, profile: ColorProfile) -> String {
    if let Some(title) = spec.title
        && run >= 4
    {
        {
            let budget = run - 3;
            let title = truncate_display(title, budget, spec.ellipsis);
            let title_width = display_width(&title);
            return format!(
                "{} {title} {}",
                spec.border_style
                    .render(&format!("{}{}", set.top_left, set.horizontal_top), profile),
                spec.border_style.render(
                    &format!(
                        "{}{}",
                        set.horizontal_top.repeat(run - title_width - 3),
                        set.top_right
                    ),
                    profile
                ),
            );
        }
    }
    spec.border_style.render(
        &format!(
            "{}{}{}",
            set.top_left,
            set.horizontal_top.repeat(run),
            set.top_right
        ),
        profile,
    )
}

/// Alignment that never invents trailing whitespace (the open-edge path).
fn align_without_trailing(line: &str, width: usize, align: Align) -> String {
    let current = display_width(line);
    if current >= width {
        return line.to_string();
    }
    let missing = width - current;
    match align {
        Align::Left => line.to_string(),
        Align::Right => format!("{}{line}", " ".repeat(missing)),
        Align::Center => format!("{}{line}", " ".repeat(missing / 2)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_have_distinct_corners() {
        assert_eq!(BorderPreset::Rounded.set().unwrap().top_left, "╭");
        assert_eq!(BorderPreset::Normal.set().unwrap().top_left, "┌");
        assert_eq!(BorderPreset::Thick.set().unwrap().top_left, "┏");
        assert_eq!(BorderPreset::Double.set().unwrap().top_left, "╔");
        let ascii = BorderPreset::Ascii.set().unwrap();
        assert_eq!(
            (ascii.top_left, ascii.horizontal_top, ascii.vertical_left),
            ("+", "-", "|")
        );
        assert!(BorderPreset::None.set().is_none());
    }

    #[test]
    fn every_preset_glyph_is_one_cell_wide() {
        for preset in [
            BorderPreset::Rounded,
            BorderPreset::Normal,
            BorderPreset::Thick,
            BorderPreset::Double,
            BorderPreset::Ascii,
        ] {
            let set = preset.set().unwrap();
            for glyph in [
                set.top_left,
                set.top_right,
                set.bottom_left,
                set.bottom_right,
                set.vertical_left,
                set.vertical_right,
                set.horizontal_top,
                set.horizontal_bottom,
            ] {
                assert_eq!(
                    crate::core::measure::display_width(glyph),
                    1,
                    "{glyph:?} in {preset:?}"
                );
            }
        }
    }

    #[test]
    fn sides_shorthand_follows_css_order() {
        assert_eq!(
            parse_sides("1").unwrap(),
            Sides {
                top: 1,
                right: 1,
                bottom: 1,
                left: 1
            }
        );
        assert_eq!(
            parse_sides("0 2").unwrap(),
            Sides {
                top: 0,
                right: 2,
                bottom: 0,
                left: 2
            }
        );
        assert_eq!(
            parse_sides("1 2 3").unwrap(),
            Sides {
                top: 1,
                right: 2,
                bottom: 3,
                left: 2
            }
        );
        assert_eq!(
            parse_sides("1 2 3 4").unwrap(),
            Sides {
                top: 1,
                right: 2,
                bottom: 3,
                left: 4
            }
        );
        assert_eq!(
            parse_sides("1,2").unwrap(),
            Sides {
                top: 1,
                right: 2,
                bottom: 1,
                left: 2
            }
        );
        assert!(parse_sides("").is_err());
        assert!(parse_sides("1 2 3 4 5").is_err());
        assert!(parse_sides("x").is_err());
    }

    use ratatui::style::Color;

    use crate::color::ColorProfile;
    use crate::core::measure::Align;
    use crate::style_spec::StyleSpec;

    fn plain(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_border_wraps_the_widest_line() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["hi", "there"]), &spec, ColorProfile::Ascii),
            plain(&["┌─────┐", "│hi   │", "│there│", "└─────┘"])
        );
    }

    #[test]
    fn padding_grows_the_box_on_each_side() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            padding: Sides {
                top: 1,
                right: 2,
                bottom: 1,
                left: 2,
            },
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["hi"]), &spec, ColorProfile::Ascii),
            plain(&["┌──────┐", "│      │", "│  hi  │", "│      │", "└──────┘"])
        );
    }

    #[test]
    fn width_pins_the_content_column_and_truncates() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            width: Some(4),
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["abcdefg"]), &spec, ColorProfile::Ascii),
            plain(&["┌────┐", "│abc…│", "└────┘"])
        );
    }

    #[test]
    fn ansi_content_keeps_the_box_square() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["\x1b[31mhi\x1b[0m"]), &spec, ColorProfile::Ascii)[1],
            "│\x1b[31mhi\x1b[0m│"
        );
    }

    #[test]
    fn align_right_pads_on_the_left() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            align: Align::Right,
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["a", "long"]), &spec, ColorProfile::Ascii),
            plain(&["┌────┐", "│   a│", "│long│", "└────┘"])
        );
    }

    #[test]
    fn the_border_style_wraps_only_the_border() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            border_style: StyleSpec {
                foreground: Some(Color::Indexed(240)),
                ..StyleSpec::default()
            },
            ..BoxSpec::default()
        };
        let out = render_box(&plain(&["hi"]), &spec, ColorProfile::Ansi256);
        assert!(out[0].starts_with("\x1b[38;5;240m┌"), "got {:?}", out[0]);
        assert_eq!(out[1], "\x1b[38;5;240m│\x1b[0mhi\x1b[38;5;240m│\x1b[0m");
    }

    #[test]
    fn the_content_style_fills_the_padding() {
        let spec = BoxSpec {
            content_style: StyleSpec {
                background: Some(Color::Indexed(212)),
                ..StyleSpec::default()
            },
            padding: Sides {
                top: 0,
                right: 1,
                bottom: 0,
                left: 1,
            },
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["hi"]), &spec, ColorProfile::Ansi256),
            plain(&["\x1b[48;5;212m hi \x1b[0m"])
        );
    }

    #[test]
    fn margin_sits_outside_the_border() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            margin: Sides {
                top: 1,
                right: 0,
                bottom: 1,
                left: 2,
            },
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["hi"]), &spec, ColorProfile::Ascii),
            plain(&["", "  ┌──┐", "  │hi│", "  └──┘", ""])
        );
    }

    #[test]
    fn a_bare_spec_is_a_passthrough() {
        assert_eq!(
            render_box(
                &plain(&["a", "longer"]),
                &BoxSpec::default(),
                ColorProfile::Ascii
            ),
            plain(&["a", "longer"])
        );
    }

    #[test]
    fn an_explicit_width_pads_even_without_a_border() {
        let spec = BoxSpec {
            width: Some(5),
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["ab"]), &spec, ColorProfile::Ascii),
            plain(&["ab   "])
        );
    }

    #[test]
    fn borders_keep_their_glyphs_under_the_ascii_profile() {
        let spec = BoxSpec {
            border: BorderPreset::Rounded,
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["x"]), &spec, ColorProfile::Ascii)[0],
            "╭─╮"
        );
    }

    #[test]
    fn a_title_sits_in_the_top_border() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            title: Some("status"),
            width: Some(12),
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["hi"]), &spec, ColorProfile::Ascii)[0],
            "┌─ status ───┐"
        );
    }

    #[test]
    fn a_long_title_is_truncated_into_the_border() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            title: Some("a very long title"),
            width: Some(8),
            ..BoxSpec::default()
        };
        let top = &render_box(&plain(&["hi"]), &spec, ColorProfile::Ascii)[0];
        assert_eq!(crate::core::measure::display_width(top), 10);
        assert!(top.contains('…'), "got {top:?}");
    }

    #[test]
    fn a_title_is_dropped_when_the_box_is_too_narrow() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            title: Some("status"),
            width: Some(2),
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["hi"]), &spec, ColorProfile::Ascii)[0],
            "┌──┐"
        );
    }

    #[test]
    fn a_title_without_a_border_is_ignored() {
        let spec = BoxSpec {
            title: Some("status"),
            ..BoxSpec::default()
        };
        assert_eq!(
            render_box(&plain(&["hi"]), &spec, ColorProfile::Ascii),
            plain(&["hi"])
        );
    }

    #[test]
    fn a_styled_title_keeps_its_escapes_without_widening_the_box() {
        let spec = BoxSpec {
            border: BorderPreset::Normal,
            title: Some("\x1b[1mrun\x1b[0m"),
            width: Some(10),
            ..BoxSpec::default()
        };
        let top = &render_box(&plain(&["hi"]), &spec, ColorProfile::Ascii)[0];
        assert_eq!(crate::core::measure::display_width(top), 12);
        assert!(top.contains("\x1b[1mrun\x1b[0m"), "got {top:?}");
    }
}
