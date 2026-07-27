// Consumed by the style box model, which lands with its wiring.
#![allow(dead_code)]

use ratatui::symbols::border::Set;

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
}
