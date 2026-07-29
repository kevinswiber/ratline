//! `rat dashboard`: N declared panes, one flicker-free frame. Thin by
//! construction — the declaration file becomes a [`Registry`], the
//! flags become a `SessionArgs`, and the watch engine does the rest.

use crate::cli::DashboardArgs;
use crate::color::ColorProfile;
use crate::commands::watch::{SessionArgs, run_registry};
use crate::core::dashboard_file::load;
use crate::core::registry::Registry;
use crate::exit::AppResult;
use crate::theme::Palette;

pub fn run(args: DashboardArgs, profile: ColorProfile, palette: Palette) -> AppResult {
    let registry = load(&args.file, args.format)?;
    let session = SessionArgs {
        once: args.once,
        clear: args.clear,
        no_hide_cursor: args.no_hide_cursor,
        no_sync: args.no_sync,
        // Declared geometry: a wrapped line would add rows the composed
        // frame's run-constant height did not budget for.
        wrap: false,
        max_height: args.max_height,
        snapshot_dir: args.snapshot_dir.clone(),
        snapshot_ansi: args.snapshot_ansi,
        live_tail: dashboard_suffix(args.once, registry.len()),
        help_heading: "rat dashboard — keys",
        help_extra: pane_help(&registry),
        // Boxes are allocated from the terminal width, so a resize
        // reflows and every child is respawned under the new geometry.
        resize_respawn: true,
    };
    run_registry(registry, session, profile, palette)
}

/// The run-constant tail of every live row — the same rule as watch's
/// `live_suffix`: nothing here may count, or the repaint gate is
/// defeated and a parked dashboard stops being byte-silent. The source
/// count is the one fact worth the width.
fn dashboard_suffix(once: bool, sources: usize) -> String {
    if once {
        return String::new();
    }
    match sources {
        1 => " · 1 source · ? help".to_string(),
        n => format!(" · {n} sources · ? help"),
    }
}

/// The dashboard's slice of the `?` reference: one line per pane with
/// its cadence, any trigger specs indented beneath — the same shape as
/// watch's trigger section.
fn pane_help(registry: &Registry) -> Vec<String> {
    let mut lines = vec![String::new(), "  panes:".to_string()];
    for id in registry.ids() {
        let spec = registry.spec(id);
        lines.push(format!(
            "    {}  {}",
            spec.name,
            crate::commands::watch::cadence_label(spec)
        ));
        for trigger in &spec.triggers {
            lines.push(format!("      {trigger}"));
        }
    }
    lines
}
