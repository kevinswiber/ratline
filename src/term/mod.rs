pub mod appearance;
pub mod buffer_ansi;
pub mod frame_state;
pub mod history;
pub mod inline;
pub mod marks;
pub mod scroll;
pub mod tab_title;
// Live on the unix input path only; the pure parts stay platform-agnostic
// so every leg keeps their unit coverage, which dead-code analysis does
// not count as use.
#[cfg_attr(windows, allow(dead_code))]
pub mod mouse;
#[cfg_attr(windows, allow(dead_code))]
pub mod tap;
#[cfg_attr(windows, allow(dead_code))]
pub mod theme_notify;
pub mod tty;
