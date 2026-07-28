pub mod appearance;
pub mod buffer_ansi;
pub mod frame_state;
pub mod inline;
#[allow(dead_code)] // Not yet wired into watch's frame loop.
pub mod scroll;
// Live on the unix input path only; the pure parts stay platform-agnostic
// so every leg keeps their unit coverage, which dead-code analysis does
// not count as use.
#[cfg_attr(windows, allow(dead_code))]
pub mod tap;
#[cfg_attr(windows, allow(dead_code))]
pub mod theme_notify;
pub mod tty;
