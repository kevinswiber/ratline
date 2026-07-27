use clap::builder::styling::{Ansi256Color, Color, Style, Styles};

use crate::color::ColorMode;

/// Help styling: bold headers, the brand pink for literals. clap only
/// applies it on a tty and honors NO_COLOR.
const HELP_STYLES: Styles = Styles::styled()
    .header(Style::new().bold())
    .usage(Style::new().bold())
    .literal(Style::new().fg_color(Some(Color::Ansi256(Ansi256Color(212)))))
    .placeholder(Style::new().dimmed());

#[derive(clap::Parser)]
#[command(
    name = "rat",
    version,
    about = "Ratatui-powered primitives for shell dashboards",
    styles = HELP_STYLES
)]
pub struct Cli {
    #[arg(long, value_enum, global = true, default_value_t = ColorMode::Auto)]
    pub color: ColorMode,
    /// Which built-in palette to use; `auto` asks the terminal
    #[arg(
        long,
        value_enum,
        global = true,
        default_value_t = crate::theme::AppearanceMode::Auto,
        env = "RAT_APPEARANCE"
    )]
    pub appearance: crate::theme::AppearanceMode,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(clap::Subcommand)]
pub enum Command {
    /// Apply colors and text attributes to text
    Style(StyleArgs),
    /// Render one-shot progress bars
    Bar(BarArgs),
    /// Align delimiter-separated rows into columns
    Table(TableArgs),
    /// Place text blocks side by side or stacked
    Join(JoinArgs),
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
    /// Test harness: print an environment variable's value, or "unset".
    #[cfg(debug_assertions)]
    #[command(name = "__env", hide = true)]
    Env(EnvArgs),
}

#[cfg(debug_assertions)]
#[derive(clap::Args)]
pub struct EnvArgs {
    pub name: String,
}

#[cfg(debug_assertions)]
#[derive(clap::Args)]
pub struct ExitCodeArgs {
    pub code: i32,
    /// Optional message printed to stderr first
    pub stderr_msg: Option<String>,
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
    /// Draw a border around the text
    #[arg(long, value_enum, default_value_t = crate::core::box_model::BorderPreset::None)]
    pub border: crate::core::box_model::BorderPreset,
    /// Border color: name, 256 index, or #rrggbb
    #[arg(long, env = "BORDER_FOREGROUND")]
    pub border_color: Option<String>,
    /// Title inserted into the top border (may be pre-styled)
    #[arg(long)]
    pub title: Option<String>,
    /// Padding inside the border: "1" | "1 2" | "1 2 3" | "1 2 3 4"
    #[arg(long)]
    pub padding: Option<String>,
    /// Margin outside the border, same shorthand as --padding
    #[arg(long)]
    pub margin: Option<String>,
    /// Content column width in display cells; longer lines truncate
    #[arg(long)]
    pub width: Option<u16>,
    /// Content alignment inside the column
    #[arg(long, value_enum, default_value_t = crate::core::measure::Align::Left)]
    pub align: crate::core::measure::Align,
    /// Marker appended to a truncated line
    #[arg(long, default_value = "…")]
    pub ellipsis: String,
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
    /// Label column width in display cells [default: 34, or the widest batch label]
    #[arg(long)]
    pub label_width: Option<u16>,
    #[arg(long, value_enum, default_value_t = crate::core::bar::BarPreset::Blocks)]
    pub preset: crate::core::bar::BarPreset,
    /// Override the fill character
    #[arg(long)]
    pub fill: Option<char>,
    /// Override the empty character
    #[arg(long)]
    pub empty: Option<char>,
    /// Fill color; wins over --thresholds when given (default: accent)
    #[arg(long)]
    pub fill_color: Option<String>,
    #[arg(long, default_value = "muted")]
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
pub struct TableArgs {
    /// Field delimiter for stdin rows
    #[arg(long, default_value_t = '\t')]
    pub delimiter: char,
    /// Per-column widths in display cells by position; empty entries auto-size ("27,,8")
    #[arg(long)]
    pub widths: Option<String>,
    /// Per-column alignment by position: l, r, or c ("l,r,r")
    #[arg(long)]
    pub align: Option<String>,
    /// Per-column overflow by position: truncate or wrap ("truncate,wrap")
    #[arg(long)]
    pub overflow: Option<String>,
    /// Text placed between columns
    #[arg(long, default_value = "  ")]
    pub separator: String,
    /// Marker appended to a truncated cell
    #[arg(long, default_value = "…")]
    pub ellipsis: String,
}

#[derive(clap::Args)]
pub struct JoinArgs {
    /// Text blocks to join, one positional argument each
    #[arg(required_unless_present = "file", conflicts_with = "file")]
    pub blocks: Vec<String>,
    /// Read a block from a file (repeatable; - reads stdin)
    #[arg(long, short = 'f')]
    pub file: Vec<std::path::PathBuf>,
    /// Join side by side (default)
    #[arg(long, conflicts_with = "vertical")]
    pub horizontal: bool,
    /// Stack blocks instead of joining side by side
    #[arg(long)]
    pub vertical: bool,
    /// Spaces (horizontal) or blank lines (vertical) between blocks
    #[arg(long, default_value_t = 0)]
    pub gap: u16,
    /// Block alignment: top/middle/bottom beside, left/center/right stacked
    #[arg(long, value_enum)]
    pub align: Option<crate::core::join::JoinAlign>,
    /// Stack vertically when the joined width exceeds the available width
    /// (RAT_WIDTH, then the terminal; no signal keeps blocks beside)
    #[arg(long, conflicts_with = "vertical")]
    pub fit: bool,
    /// Available width for --fit in display cells (implies --fit)
    #[arg(long, conflicts_with = "vertical")]
    pub max_width: Option<u16>,
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
pub struct SparkArgs {
    /// Numbers to plot; reads stdin when omitted
    #[arg(allow_negative_numbers = true)]
    pub values: Vec<String>,
    /// Clamp the lower bound
    #[arg(long, allow_negative_numbers = true)]
    pub min: Option<f64>,
    /// Clamp the upper bound
    #[arg(long, allow_negative_numbers = true)]
    pub max: Option<f64>,
    /// Color for the sparkline
    #[arg(long = "spark-color", visible_alias = "color-fg")]
    pub spark_color: Option<String>,
}

#[derive(clap::Args)]
pub struct LogArgs {
    /// Message text; joined with single spaces
    pub text: Vec<String>,
    /// Log level tag
    #[arg(short = 'l', long, value_enum)]
    pub level: Option<LogLevel>,
    /// Minimum level to emit (lower levels are dropped)
    #[arg(long, value_enum, env = "RAT_LOG_LEVEL")]
    pub min_level: Option<LogLevel>,
    /// Prefix with the current time in this strftime format
    #[arg(short = 't', long)]
    pub time: Option<String>,
    /// Append plain text to this file instead of stderr
    #[arg(short = 'o', long)]
    pub file: Option<std::path::PathBuf>,
}

#[derive(Copy, Clone, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

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
    /// Clear the screen before the first frame (atomically, inside it)
    #[arg(long)]
    pub clear: bool,
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
pub struct ChooseArgs {
    /// Options to pick from; reads stdin when omitted
    pub options: Vec<String>,
    /// Maximum selections (single-select by default)
    #[arg(long, default_value_t = 1, conflicts_with = "no_limit")]
    pub limit: usize,
    /// Allow any number of selections
    #[arg(long)]
    pub no_limit: bool,
    /// Return results in list order instead of selection order
    #[arg(long)]
    pub ordered: bool,
    /// Visible list height
    #[arg(long, default_value_t = 10)]
    pub height: u16,
    /// Cursor marker
    #[arg(long, default_value = "> ")]
    pub cursor: String,
    /// Header shown above the list
    #[arg(long, default_value = "Choose:")]
    pub header: String,
    /// Prefix for selected items (multi-select)
    #[arg(long, default_value = "✓ ")]
    pub selected_prefix: String,
    /// Prefix for unselected items (multi-select)
    #[arg(long, default_value = "• ")]
    pub unselected_prefix: String,
    /// Preselect these options
    #[arg(long)]
    pub selected: Vec<String>,
    /// Auto-select when only one option exists
    #[arg(long)]
    pub select_if_one: bool,
    /// Delimiter for stdin options
    #[arg(long, default_value = "\n")]
    pub input_delimiter: String,
    /// Delimiter joining printed results
    #[arg(long, default_value = "\n")]
    pub output_delimiter: String,
    /// Show the key help footer
    #[arg(long, overrides_with = "no_show_help")]
    pub show_help: bool,
    #[arg(long)]
    pub no_show_help: bool,
    /// Give up after this long (exit 124)
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(clap::Args)]
pub struct ConfirmArgs {
    /// The question to ask
    #[arg(default_value = "Are you sure?")]
    pub prompt: String,
    /// Initially selected answer
    #[arg(long = "default", default_value_t = true, action = clap::ArgAction::Set)]
    pub default_yes: bool,
    /// Label for the affirmative answer
    #[arg(long, default_value = "Yes")]
    pub affirmative: String,
    /// Label for the negative answer
    #[arg(long, default_value = "No")]
    pub negative: String,
    /// Print the chosen label to stdout
    #[arg(long)]
    pub show_output: bool,
    /// Give up after this long (exit 124)
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(clap::Args)]
pub struct InputArgs {
    /// Placeholder shown while empty
    #[arg(long, default_value = "Type something...")]
    pub placeholder: String,
    /// Prompt prefix
    #[arg(long, default_value = "> ")]
    pub prompt: String,
    /// Initial value
    #[arg(long, default_value = "")]
    pub value: String,
    /// Mask the input
    #[arg(long)]
    pub password: bool,
    /// Maximum input length
    #[arg(long, default_value_t = 400)]
    pub char_limit: usize,
    /// Header line above the prompt
    #[arg(long)]
    pub header: Option<String>,
    /// Give up after this long (exit 124)
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(clap::Args)]
pub struct FilterArgs {
    /// Maximum selections (single-select by default)
    #[arg(long, default_value_t = 1, conflicts_with = "no_limit")]
    pub limit: usize,
    /// Allow any number of selections
    #[arg(long)]
    pub no_limit: bool,
    /// Placeholder shown while the query is empty
    #[arg(long, default_value = "Filter...")]
    pub placeholder: String,
    /// Prompt prefix
    #[arg(long, default_value = "> ")]
    pub prompt: String,
    /// Initial query
    #[arg(long, default_value = "")]
    pub value: String,
    /// Visible list height
    #[arg(long, default_value_t = 10)]
    pub height: u16,
    /// With no matches, print nothing instead of the query
    #[arg(long, overrides_with = "no_strict", default_value_t = true)]
    pub strict: bool,
    #[arg(long)]
    pub no_strict: bool,
    /// Fuzzy matching (subsequences) instead of substring
    #[arg(long, overrides_with = "no_fuzzy", default_value_t = true)]
    pub fuzzy: bool,
    #[arg(long)]
    pub no_fuzzy: bool,
    /// Rank matches by score
    #[arg(long, overrides_with = "no_fuzzy_sort", default_value_t = true)]
    pub fuzzy_sort: bool,
    #[arg(long)]
    pub no_fuzzy_sort: bool,
    /// Cursor indicator
    #[arg(long, default_value = "• ")]
    pub indicator: String,
    /// Prefix for selected items (multi-select)
    #[arg(long, default_value = "◉ ")]
    pub selected_prefix: String,
    /// Prefix for unselected items (multi-select)
    #[arg(long, default_value = "○ ")]
    pub unselected_prefix: String,
    /// Header line above the prompt
    #[arg(long)]
    pub header: Option<String>,
    /// Auto-select when only one candidate exists
    #[arg(long)]
    pub select_if_one: bool,
    /// Delimiter for stdin candidates
    #[arg(long, default_value = "\n")]
    pub input_delimiter: String,
    /// Delimiter joining printed results
    #[arg(long, default_value = "\n")]
    pub output_delimiter: String,
    /// Give up after this long (exit 124)
    #[arg(long)]
    pub timeout: Option<String>,
}

#[derive(clap::Args)]
pub struct SpinArgs {
    /// Spinner charset
    #[arg(short = 's', long, value_enum, default_value_t = crate::ui::spin::Spinner::Dot)]
    pub spinner: crate::ui::spin::Spinner,
    /// Text shown beside the spinner
    #[arg(long, default_value = "Loading...")]
    pub title: String,
    /// Show the child's stdout and stderr when it finishes
    #[arg(long)]
    pub show_output: bool,
    /// Show only the child's stdout
    #[arg(long)]
    pub show_stdout: bool,
    /// Show only the child's stderr
    #[arg(long)]
    pub show_stderr: bool,
    /// Show the child's output only when it fails
    #[arg(long)]
    pub show_error: bool,
    /// Kill the child after this long (exit 124)
    #[arg(long)]
    pub timeout: Option<String>,
    /// Command to run (after --)
    #[arg(last = true, required = true)]
    pub command: Vec<String>,
}

#[derive(clap::Args)]
pub struct CompletionArgs {
    /// Shell to generate completions for
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

#[cfg(test)]
mod tests {
    use clap::{CommandFactory, Parser};

    #[test]
    fn cli_is_well_formed() {
        super::Cli::command().debug_assert();
    }

    #[test]
    fn the_appearance_flag_parses_every_mode() {
        // An explicit flag outranks the environment, so this is stable no
        // matter what the developer's shell exports.
        for (arg, expected) in [
            ("auto", crate::theme::AppearanceMode::Auto),
            ("light", crate::theme::AppearanceMode::Light),
            ("dark", crate::theme::AppearanceMode::Dark),
        ] {
            let cli = super::Cli::parse_from(["rat", "--appearance", arg, "style", "x"]);
            assert_eq!(cli.appearance, expected);
        }
    }

    #[test]
    fn table_ellipsis_default_matches_the_measure_constant() {
        // clap's default_value wants a literal; this keeps the two in sync.
        let cli = super::Cli::parse_from(["rat", "table"]);
        let super::Command::Table(args) = cli.command else {
            panic!("expected the table subcommand");
        };
        assert_eq!(args.ellipsis, crate::core::measure::ELLIPSIS);
    }
}
