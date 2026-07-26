use std::str::FromStr;

use anyhow::anyhow;
use ratatui::style::Color;

use crate::color::ColorProfile;

/// Text attributes plus colors, rendered as one SGR sequence.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StyleSpec {
    pub bold: bool,
    pub faint: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Layer {
    Fg,
    Bg,
}

impl StyleSpec {
    pub fn sgr_prefix(&self, profile: ColorProfile) -> String {
        if profile == ColorProfile::Ascii {
            return String::new();
        }
        let mut parts: Vec<String> = Vec::new();
        for (on, code) in [
            (self.bold, "1"),
            (self.faint, "2"),
            (self.italic, "3"),
            (self.underline, "4"),
            (self.strikethrough, "9"),
        ] {
            if on {
                parts.push(code.to_string());
            }
        }
        if let Some(part) = self
            .foreground
            .and_then(|c| color_sgr(c, profile, Layer::Fg))
        {
            parts.push(part);
        }
        if let Some(part) = self
            .background
            .and_then(|c| color_sgr(c, profile, Layer::Bg))
        {
            parts.push(part);
        }
        parts.join(";")
    }

    pub fn render(&self, text: &str, profile: ColorProfile) -> String {
        let prefix = self.sgr_prefix(profile);
        if prefix.is_empty() {
            text.to_string()
        } else {
            format!("\x1b[{prefix}m{text}\x1b[0m")
        }
    }
}

/// Accepts named colors, 256 indices, and #rrggbb hex — a superset of gum,
/// which silently drops named colors.
pub fn parse_color(s: &str) -> anyhow::Result<Color> {
    Color::from_str(s).map_err(|_| anyhow!("invalid color: {s}"))
}

pub fn color_sgr(c: Color, profile: ColorProfile, layer: Layer) -> Option<String> {
    if profile == ColorProfile::Ascii {
        return None;
    }
    let named = |idx: u8| Some(basic_sgr(idx, layer));
    match c {
        Color::Reset => None,
        Color::Black => named(0),
        Color::Red => named(1),
        Color::Green => named(2),
        Color::Yellow => named(3),
        Color::Blue => named(4),
        Color::Magenta => named(5),
        Color::Cyan => named(6),
        Color::Gray => named(7),
        Color::DarkGray => named(8),
        Color::LightRed => named(9),
        Color::LightGreen => named(10),
        Color::LightYellow => named(11),
        Color::LightBlue => named(12),
        Color::LightMagenta => named(13),
        Color::LightCyan => named(14),
        Color::White => named(15),
        Color::Indexed(n) => match profile {
            ColorProfile::Ansi => Some(basic_sgr(n & 0x0f, layer)),
            _ => Some(format!("{};5;{n}", extended_base(layer))),
        },
        Color::Rgb(r, g, b) => match profile {
            ColorProfile::TrueColor => Some(format!("{};2;{r};{g};{b}", extended_base(layer))),
            ColorProfile::Ansi256 => Some(format!(
                "{};5;{}",
                extended_base(layer),
                rgb_to_256(r, g, b)
            )),
            _ => Some(basic_sgr(rgb_to_16(r, g, b), layer)),
        },
    }
}

fn extended_base(layer: Layer) -> u8 {
    match layer {
        Layer::Fg => 38,
        Layer::Bg => 48,
    }
}

fn basic_sgr(idx: u8, layer: Layer) -> String {
    let code = match (idx < 8, layer) {
        (true, Layer::Fg) => 30 + idx,
        (true, Layer::Bg) => 40 + idx,
        (false, Layer::Fg) => 90 + (idx - 8),
        (false, Layer::Bg) => 100 + (idx - 8),
    };
    code.to_string()
}

/// Nearest xterm 256-palette index: the 6x6x6 color cube or the gray ramp.
fn rgb_to_256(r: u8, g: u8, b: u8) -> u8 {
    if r == g && g == b {
        if r < 8 {
            return 16;
        }
        if r > 248 {
            return 231;
        }
        return 232 + ((u16::from(r) - 8) / 10) as u8;
    }
    let level = |v: u8| -> u8 {
        if v < 48 {
            0
        } else if v < 115 {
            1
        } else {
            ((u16::from(v) - 35) / 40) as u8
        }
    };
    16 + 36 * level(r) + 6 * level(g) + level(b)
}

/// Nearest of the 16 xterm default colors by squared RGB distance.
fn rgb_to_16(r: u8, g: u8, b: u8) -> u8 {
    const PALETTE: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    let dist = |(pr, pg, pb): (u8, u8, u8)| -> u32 {
        let d = |a: u8, b: u8| {
            let diff = i32::from(a) - i32::from(b);
            (diff * diff) as u32
        };
        d(pr, r) + d(pg, g) + d(pb, b)
    };
    PALETTE
        .iter()
        .enumerate()
        .min_by_key(|(_, rgb)| dist(**rgb))
        .map(|(i, _)| i as u8)
        .unwrap_or(7)
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::*;
    use crate::color::ColorProfile;

    fn spec_bold_212() -> StyleSpec {
        StyleSpec {
            bold: true,
            foreground: Some(Color::Indexed(212)),
            ..StyleSpec::default()
        }
    }

    #[test]
    fn ascii_renders_plain() {
        assert_eq!(spec_bold_212().render("X", ColorProfile::Ascii), "X");
    }

    #[test]
    fn empty_spec_renders_plain() {
        assert_eq!(
            StyleSpec::default().render("X", ColorProfile::TrueColor),
            "X"
        );
    }

    #[test]
    fn bold_indexed_fg_under_ansi256() {
        assert_eq!(
            spec_bold_212().render("X", ColorProfile::Ansi256),
            "\x1b[1;38;5;212mX\x1b[0m"
        );
    }

    #[test]
    fn rgb_under_truecolor() {
        assert_eq!(
            color_sgr(Color::Rgb(255, 0, 0), ColorProfile::TrueColor, Layer::Fg),
            Some("38;2;255;0;0".to_string())
        );
    }

    #[test]
    fn rgb_downsamples_to_256_cube() {
        assert_eq!(
            color_sgr(Color::Rgb(255, 0, 0), ColorProfile::Ansi256, Layer::Fg),
            Some("38;5;196".to_string())
        );
    }

    #[test]
    fn rgb_downsamples_to_basic_under_ansi() {
        assert_eq!(
            color_sgr(Color::Rgb(255, 0, 0), ColorProfile::Ansi, Layer::Fg),
            Some("91".to_string())
        );
    }

    #[test]
    fn named_red_is_31() {
        assert_eq!(
            color_sgr(Color::Red, ColorProfile::Ansi, Layer::Fg),
            Some("31".to_string())
        );
    }

    #[test]
    fn indexed_under_ansi_masks_to_16() {
        assert_eq!(
            color_sgr(Color::Indexed(212), ColorProfile::Ansi, Layer::Fg),
            Some("34".to_string()) // 212 & 0x0f == 4 -> blue
        );
    }

    #[test]
    fn background_uses_bg_codes() {
        assert_eq!(
            color_sgr(Color::Indexed(212), ColorProfile::Ansi256, Layer::Bg),
            Some("48;5;212".to_string())
        );
        assert_eq!(
            color_sgr(Color::Red, ColorProfile::Ansi, Layer::Bg),
            Some("41".to_string())
        );
    }

    #[test]
    fn parse_color_accepts_named_index_and_hex() {
        assert_eq!(parse_color("red").unwrap(), Color::Red);
        assert_eq!(parse_color("212").unwrap(), Color::Indexed(212));
        assert_eq!(parse_color("#ff0000").unwrap(), Color::Rgb(255, 0, 0));
        assert!(parse_color("definitely-not-a-color").is_err());
    }

    #[test]
    fn all_attributes_in_prefix() {
        let spec = StyleSpec {
            bold: true,
            faint: true,
            italic: true,
            underline: true,
            strikethrough: true,
            ..StyleSpec::default()
        };
        assert_eq!(spec.sgr_prefix(ColorProfile::Ansi), "1;2;3;4;9");
        assert_eq!(spec.sgr_prefix(ColorProfile::Ascii), "");
    }
}
