use crate::color::ColorMode;

#[derive(clap::Parser)]
#[command(
    name = "rat",
    version,
    about = "Ratatui-powered primitives for shell dashboards"
)]
pub struct Cli {
    #[arg(long, value_enum, global = true, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Apply colors and text attributes to text
    Style(StyleArgs),
    /// Render one-shot progress bars
    Bar(BarArgs),
    /// Format and parse durations
    Duration(DurationArgs),
    /// Parse, format, and diff timestamps portably
    Date(DateArgs),
    /// Render a sparkline from numbers
    Spark(SparkArgs),
    /// Print a styled log line to stderr
    Log(LogArgs),
    /// Repaint a frame of output in place, flicker-free
    Frame(FrameArgs),
    /// Run a command on an interval and repaint its output flicker-free
    Watch(WatchArgs),
    /// Diagnose terminal capabilities
    Doctor(DoctorArgs),
    /// Pick one or more items from a list
    Choose(ChooseArgs),
    /// Ask a yes/no question
    Confirm(ConfirmArgs),
    /// Prompt for a line of input
    Input(InputArgs),
    /// Fuzzy-filter lines from stdin
    Filter(FilterArgs),
    /// Show a spinner while a command runs
    Spin(SpinArgs),
    /// Generate shell completions
    Completion(CompletionArgs),
    /// Test harness: exit with the given code through AppError mapping.
    #[cfg(debug_assertions)]
    #[command(name = "__exitcode", hide = true)]
    ExitCode(ExitCodeArgs),
}

#[cfg(debug_assertions)]
#[derive(clap::Args)]
pub struct ExitCodeArgs {
    pub code: i32,
}

#[derive(clap::Args)]
pub struct StyleArgs {
    /// Text to style; reads stdin when omitted. Multiple args join with newlines.
    pub text: Vec<String>,
    #[arg(long)]
    pub bold: bool,
    #[arg(long)]
    pub faint: bool,
    #[arg(long)]
    pub italic: bool,
    #[arg(long)]
    pub underline: bool,
    #[arg(long)]
    pub strikethrough: bool,
    /// Foreground color: name, 256 index, or #rrggbb
    #[arg(long, env = "FOREGROUND")]
    pub foreground: Option<String>,
    /// Background color: name, 256 index, or #rrggbb
    #[arg(long, env = "BACKGROUND")]
    pub background: Option<String>,
    /// Trim whitespace from each line
    #[arg(long)]
    pub trim: bool,
    /// Strip ANSI escapes from input before styling (default)
    #[arg(long, overrides_with = "no_strip_ansi")]
    pub strip_ansi: bool,
    /// Keep ANSI escapes present in the input
    #[arg(long)]
    pub no_strip_ansi: bool,
}

#[derive(clap::Args)]
pub struct BarArgs {
    /// Current value
    #[arg(long, allow_negative_numbers = true)]
    pub value: Option<f64>,
    #[arg(long, default_value_t = 100.0, allow_negative_numbers = true)]
    pub total: f64,
    /// Bar width in cells
    #[arg(long, default_value_t = 32)]
    pub width: u16,
    #[arg(long)]
    pub label: Option<String>,
    /// Label column width (display cells)
    #[arg(long, default_value_t = 34)]
    pub label_width: u16,
    #[arg(long, value_enum, default_value_t = crate::core::bar::BarPreset::Blocks)]
    pub preset: crate::core::bar::BarPreset,
    /// Override the fill character
    #[arg(long)]
    pub fill: Option<char>,
    /// Override the empty character
    #[arg(long)]
    pub empty: Option<char>,
    /// Fill color; wins over --thresholds when given (default 212)
    #[arg(long)]
    pub fill_color: Option<String>,
    #[arg(long, default_value = "240")]
    pub empty_color: String,
    /// State word appended after the annotations
    #[arg(long)]
    pub state: Option<String>,
    #[arg(long, value_enum, default_value_t = crate::core::bar::Annotation::Both)]
    pub annotation: crate::core::bar::Annotation,
    /// Color the fill by percentage band, e.g. "33:196,66:214,100:42"
    #[arg(long)]
    pub thresholds: Option<String>,
    /// Field delimiter for stdin batch rows
    #[arg(long, default_value_t = '\t')]
    pub delimiter: char,
    /// Render a moving block instead of progress (unknown total)
    #[arg(long)]
    pub indeterminate: bool,
    /// Animation step for --indeterminate
    #[arg(long, default_value_t = 0)]
    pub tick: u64,
}

#[derive(clap::Args)]
pub struct DurationArgs {
    /// Seconds to format, or a duration string with --seconds
    pub value: String,
    /// Parse a duration string ("1h33m") and print integer seconds
    #[arg(long, conflicts_with_all = ["ms", "format"])]
    pub seconds: bool,
    /// Treat the value as milliseconds
    #[arg(long)]
    pub ms: bool,
    #[arg(long, value_enum, default_value_t = crate::core::duration::DurationFormat::Compact)]
    pub format: crate::core::duration::DurationFormat,
}

#[derive(clap::Args)]
pub struct DateArgs {
    /// "now" (default), epoch seconds, or an RFC3339 timestamp
    pub value: Option<String>,
    /// Print epoch seconds
    #[arg(long, conflicts_with_all = ["format", "relative", "since", "until"])]
    pub epoch: bool,
    /// strftime output format
    #[arg(long)]
    pub format: Option<String>,
    /// Use UTC instead of the local zone
    #[arg(long)]
    pub utc: bool,
    /// Phrase the timestamp relative to now
    #[arg(long, conflicts_with_all = ["format", "since", "until"])]
    pub relative: bool,
    /// Seconds elapsed from this timestamp to the value
    #[arg(long, conflicts_with = "until")]
    pub since: Option<String>,
    /// Seconds remaining from the value to this timestamp
    #[arg(long)]
    pub until: Option<String>,
}

#[derive(clap::Args)]
pub struct SparkArgs {}

#[derive(clap::Args)]
pub struct LogArgs {}

#[derive(clap::Args)]
pub struct FrameArgs {
    #[command(subcommand)]
    pub action: Option<FrameAction>,
    /// State file path (defaults to a per-terminal temp file)
    #[arg(long)]
    pub state: Option<std::path::PathBuf>,
    /// Override the detected terminal width
    #[arg(long)]
    pub width: Option<u16>,
    /// Skip synchronized-output escapes
    #[arg(long)]
    pub no_sync: bool,
    /// Leave the cursor visible while painting
    #[arg(long)]
    pub no_hide_cursor: bool,
    /// Forget the previous frame without painting
    #[arg(long, conflicts_with_all = ["finish", "clear"])]
    pub reset: bool,
    /// Show the cursor, close any open frame, and forget state
    #[arg(long, conflicts_with = "clear")]
    pub finish: bool,
    /// Erase the painted frame, show the cursor, and forget state
    #[arg(long)]
    pub clear: bool,
}

#[derive(clap::Subcommand)]
pub enum FrameAction {
    /// Emit the begin-synchronized-update escape
    Begin,
    /// Emit the end-synchronized-update escape
    End,
}

#[derive(clap::Args)]
pub struct WatchArgs {
    /// Refresh interval, e.g. "2s", "500ms", "1m"
    #[arg(short = 'n', long, default_value = "2s")]
    pub interval: String,
    /// Run one tick and exit
    #[arg(long)]
    pub once: bool,
    /// Leave the cursor visible
    #[arg(long)]
    pub no_hide_cursor: bool,
    /// Skip synchronized-output escapes
    #[arg(long)]
    pub no_sync: bool,
    /// Run the command through `sh -c`
    #[arg(long)]
    pub shell: bool,
    /// Bold title line above the output
    #[arg(long)]
    pub title: Option<String>,
    /// Cap the painted height (defaults to terminal height minus two)
    #[arg(long)]
    pub max_height: Option<u16>,
    /// Command to run each tick (after --)
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

#[derive(clap::Args)]
pub struct DoctorArgs {
    /// Machine-readable output
    #[arg(long)]
    pub json: bool,
}

#[derive(clap::Args)]
pub struct ChooseArgs {}

#[derive(clap::Args)]
pub struct ConfirmArgs {}

#[derive(clap::Args)]
pub struct InputArgs {}

#[derive(clap::Args)]
pub struct FilterArgs {}

#[derive(clap::Args)]
pub struct SpinArgs {}

#[derive(clap::Args)]
pub struct CompletionArgs {}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    #[test]
    fn cli_is_well_formed() {
        super::Cli::command().debug_assert();
    }
}
