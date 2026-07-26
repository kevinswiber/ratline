/// Spinner charsets matching gum's names.
#[derive(Copy, Clone, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum Spinner {
    Line,
    Dot,
    Minidot,
    Jump,
    Pulse,
    Points,
    Globe,
    Moon,
    Monkey,
    Meter,
    Hamburger,
}

impl Spinner {
    pub fn frames(self) -> &'static [&'static str] {
        match self {
            Spinner::Line => &["-", "\\", "|", "/"],
            Spinner::Dot => &["⣾", "⣽", "⣻", "⢿", "⡿", "⣟", "⣯", "⣷"],
            Spinner::Minidot => &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"],
            Spinner::Jump => &["⢄", "⢂", "⢁", "⡁", "⡈", "⡐", "⡠"],
            Spinner::Pulse => &["█", "▓", "▒", "░"],
            Spinner::Points => &["∙∙∙", "●∙∙", "∙●∙", "∙∙●"],
            Spinner::Globe => &["🌍", "🌎", "🌏"],
            Spinner::Moon => &["🌑", "🌒", "🌓", "🌔", "🌕", "🌖", "🌗", "🌘"],
            Spinner::Monkey => &["🙈", "🙉", "🙊"],
            Spinner::Meter => &["▱▱▱", "▰▱▱", "▰▰▱", "▰▰▰", "▰▰▱", "▰▱▱", "▱▱▱"],
            Spinner::Hamburger => &["☱", "☲", "☴"],
        }
    }

    pub fn frame(self, tick: u64) -> &'static str {
        let frames = self.frames();
        frames[(tick % frames.len() as u64) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spinner_has_frames_and_cycles() {
        for spinner in [
            Spinner::Line,
            Spinner::Dot,
            Spinner::Minidot,
            Spinner::Jump,
            Spinner::Pulse,
            Spinner::Points,
            Spinner::Globe,
            Spinner::Moon,
            Spinner::Monkey,
            Spinner::Meter,
            Spinner::Hamburger,
        ] {
            let frames = spinner.frames();
            assert!(!frames.is_empty());
            assert_eq!(spinner.frame(0), frames[0]);
            assert_eq!(spinner.frame(frames.len() as u64), frames[0]);
        }
    }
}
