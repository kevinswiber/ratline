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
pub struct StyleArgs {}

#[derive(clap::Args)]
pub struct BarArgs {}

#[derive(clap::Args)]
pub struct DurationArgs {}

#[derive(clap::Args)]
pub struct DateArgs {}

#[derive(clap::Args)]
pub struct SparkArgs {}

#[derive(clap::Args)]
pub struct LogArgs {}

#[derive(clap::Args)]
pub struct FrameArgs {}

#[derive(clap::Args)]
pub struct WatchArgs {}

#[derive(clap::Args)]
pub struct DoctorArgs {}

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
