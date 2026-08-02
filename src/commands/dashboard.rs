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
    // A load error prints before any UI exists, so the profile is the
    // one color authority it gets: anything above Ascii earns the
    // colored snippet theme.
    let registry = load(&args.file, profile != ColorProfile::Ascii)?;
    let once_timeout = args
        .once_timeout
        .as_deref()
        .map(crate::core::duration::parse_interval)
        .transpose()
        .map_err(|err| anyhow::anyhow!("--once-timeout: {err:#}"))?;
    let session = SessionArgs {
        once: args.once,
        // The declaration file's stem is the tab's fallback identity;
        // the declared title (static or pane-sourced) outranks it at
        // emit time.
        tab_title: Some(
            args.file
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| "dashboard".to_string()),
        ),
        once_timeout,
        clear: args.clear,
        fullscreen: args.fullscreen,
        mouse: args.mouse,
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
            spec.id,
            crate::commands::watch::cadence_label(spec)
        ));
        for trigger in &spec.triggers {
            lines.push(format!("      {trigger}"));
        }
    }
    if registry.ids().any(|id| registry.spec(id).live) {
        lines.extend(LIVE_HELP.iter().map(|l| (*l).to_string()));
    }
    if registry
        .ids()
        .any(|id| !registry.spec(id).triggers.is_empty())
    {
        lines.extend(LOOPING_HELP.iter().map(|l| (*l).to_string()));
    }
    // The diagnostics channel: load-time facts worth telling but not
    // worth failing over. Absent diagnostics add NOTHING — the plain
    // help stays byte-identical.
    if !registry.diagnostics().is_empty() {
        lines.push(String::new());
        lines.push("  diagnostics:".to_string());
        for diagnostic in registry.diagnostics() {
            // The pager has no wrapping engine; the key table's width
            // is the budget every help line keeps.
            lines.push(crate::core::measure::truncate_display(
                &format!("    {diagnostic}"),
                74,
                crate::core::measure::ELLIPSIS,
            ));
        }
    }
    lines
}

/// What the `live` label means, and what `interval` means under it.
///
/// **Static, like the looping section below**: `pane_help` runs once at
/// startup, so this states the contract rather than live state.
///
/// The `interval` sentence is the load-bearing one and is not guessable:
/// on a live pane the interval does nothing while the child runs, and
/// becomes the delay before a replacement spawns if it exits. Written as
/// what the knob DOES today, not as a promise that it is the only knob
/// there will ever be — a dedicated backoff can join it without making
/// this section a lie.
///
/// Wrapped by hand: `?` pages plain grouped text through the pager, so
/// there is no wrapping engine to lean on, and nothing here may exceed
/// the width the key table already sets.
const LIVE_HELP: &[&str] = &[
    "",
    "  live panes:",
    "    A pane marked `live` runs one long-lived command, spawned once",
    "    and painted as it prints — it is not on a cadence. Its",
    "    `interval` is how soon a replacement spawns if the child exits,",
    "    not how often it runs. `interval \"never\"` means no replacement:",
    "    the pane keeps its exit badge. A `trigger` on a live pane",
    "    restarts the child: the running one is killed and a replacement",
    "    spawned in its place.",
];

/// What `· looping` means, and what to do about it.
///
/// **Static, and it does not name the looping panes.** `pane_help` is
/// called once at startup and its result is stored in the session, so
/// `?` cannot report live state without making help dynamic — a bigger
/// change than this earns. The live naming is the notice row's job; it
/// already names the panes and the paths. This is a deliberate
/// departure from the sketch, which showed `?` listing the specific
/// panes, and it is recorded here so it does not read as an oversight.
///
/// **The closing paragraph is about what the badge's ABSENCE does not
/// mean**, and it is here because the detector can now decline to answer.
/// A window abstains when a candidate's reader evidence is all ambiguous,
/// and condition 3 has always abstained on lost evidence or a busy
/// dashboard — none of which reach a surface. So a silent dashboard and a
/// clean one look identical from outside, which is a quieter form of the
/// confident-negative defect the interval model removed.
///
/// Deliberately STATIC, and deliberately not a live "cannot decide" badge.
/// Nobody has measured how often abstention actually fires, and
/// `Verdict::abstained` carries no user-facing cause to put in a notice —
/// so a dynamic surface would be guessing at both its frequency and its
/// wording. Stating the limitation once, where the badge is explained,
/// costs nothing and is true today.
///
/// The mtime sentence below is the load-bearing one and is not guessable:
/// `fingerprint` is mtime-only, so a command that rewrites a file with
/// identical bytes still fires the trigger. Guarding the *change*
/// rather than the *write* is the mistake that produces exactly this
/// badge, and it is a real reported confusion elsewhere, not a
/// hypothetical one.
///
/// Wrapped by hand: `?` pages plain grouped text through the pager, so
/// there is no wrapping engine to lean on, and nothing here may exceed
/// the width the key table already sets.
const LOOPING_HELP: &[&str] = &[
    "",
    "  looping panes:",
    "    A pane marked `· looping` is still running — nothing has been",
    "    stopped. rat cannot see who writes a file, only that a watched",
    "    path changes while the dashboard is busy and never while it is",
    "    idle, which is what a pane whose own command touches another",
    "    pane's trigger looks like.",
    "",
    "    The fix is in the declaration: give the command a guard so it",
    "    writes only when the content changed, or point the trigger at a",
    "    path no pane writes. A trigger fires on mtime, not on content,",
    "    so writing identical bytes still fires — the guard has to skip",
    "    the write, not just the change.",
    "",
    "    The absence of the badge is weaker than its presence. rat stays",
    "    silent whenever it cannot tell: when a write cannot be placed",
    "    against the commands that were running, when a reader's evidence",
    "    was lost, or when the dashboard was too busy to judge. No badge",
    "    means no loop was proved, not that there is none.",
];

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::box_model::{BorderPreset, Sides};
    use crate::core::registry::{LayoutNode, Overflow, PaneBox, PaneWidth, SourceId, SourceSpec};
    use crate::core::trigger::TriggerSpec;

    fn registry(triggers: bool) -> Registry {
        let spec = |id: &str, path: &str| SourceSpec {
            id: id.to_string(),
            command: vec!["true".to_string()],
            shell: false,
            interval: (!triggers).then(|| Duration::from_secs(5)),
            triggers: if triggers {
                vec![TriggerSpec::File(std::path::PathBuf::from(path))]
            } else {
                Vec::new()
            },
            debounce: Duration::from_millis(250),
            live: false,
        };
        let pane = || PaneBox {
            height: 5,
            width: PaneWidth::Weight(1),
            overflow: Overflow::KeepTop,
            border: BorderPreset::Rounded,
            padding: Sides::default(),
            title: None,
            chrome: true,
        };
        Registry::panes(
            vec![spec("a", "./sa"), spec("b", "./sb")],
            vec![pane(), pane()],
            LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
            ]),
            1,
            0,
        )
        .expect("a valid two-pane registry")
    }

    /// One live follower beside one batch pane — the mixed dashboard
    /// the help must serve. The live pane keeps its tail, as load
    /// would have enforced.
    fn live_registry() -> Registry {
        let batch = SourceSpec {
            id: "a".to_string(),
            command: vec!["true".to_string()],
            shell: false,
            interval: Some(Duration::from_secs(5)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(250),
            live: false,
        };
        let follow = SourceSpec {
            id: "follow".to_string(),
            command: vec!["true".to_string()],
            shell: false,
            interval: Some(Duration::from_secs(2)),
            triggers: Vec::new(),
            debounce: Duration::from_millis(250),
            live: true,
        };
        let pane = |overflow| PaneBox {
            height: 5,
            width: PaneWidth::Weight(1),
            overflow,
            border: BorderPreset::Rounded,
            padding: Sides::default(),
            title: None,
            chrome: true,
        };
        Registry::panes(
            vec![batch, follow],
            vec![pane(Overflow::KeepTop), pane(Overflow::KeepBottom)],
            LayoutNode::Row(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
            ]),
            1,
            0,
        )
        .expect("a valid two-pane registry")
    }

    #[test]
    fn the_help_explains_what_interval_means_on_a_live_pane() {
        // The owner contract, stated where a chrome row has no room:
        // `interval` on a live pane is the respawn delay. Not a
        // guessable fact — nothing on the pane says it. De-wrapped
        // before matching, for the same reason as the looping test.
        let lines = pane_help(&live_registry());
        assert!(
            lines.contains(&"    follow  live".to_string()),
            "the pane list must carry the live label: {lines:?}"
        );
        let text = lines.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        assert!(text.contains("live panes:"), "got {text}");
        assert!(text.contains("how soon a replacement spawns"), "got {text}");
        assert!(text.contains("not how often it runs"), "got {text}");
        // And the opt-out, which is the same contract's other half:
        // `interval "never"` on a live pane means no replacement.
        assert!(text.contains("no replacement"), "got {text}");
        // And the trigger contract: a fire restarts the child. The
        // README points at `?` for this, so `?` must actually say it.
        assert!(text.contains("restarts the child"), "got {text}");
    }

    #[test]
    fn the_live_help_is_absent_when_no_pane_is_live() {
        // The trigger-only dashboard must not gain the live section —
        // and the no-trigger, no-live case is already pinned
        // byte-identical below.
        let text = pane_help(&registry(true)).join(" ");
        assert!(!text.contains("live panes:"), "got {text}");
    }

    #[test]
    fn the_help_explains_the_badge_and_names_the_fix() {
        // De-wrapped before matching. The section is wrapped by hand,
        // so a phrase the reader sees as one sentence can straddle a
        // line break — asserting on the wrapped bytes would pin the
        // wrap POINTS rather than the claim, and would fail on any
        // future rewrap that changed nothing a user can perceive. The
        // width is pinned separately, below, because that is a
        // different property.
        let lines = pane_help(&registry(true));
        let text = lines.iter().map(|l| l.trim()).collect::<Vec<_>>().join(" ");
        assert!(text.contains("looping"), "got {text}");
        // The two facts a user cannot guess: that the pane was left
        // running, and that a trigger fires on the timestamp rather
        // than on the bytes — so writing identical content still fires.
        assert!(text.contains("nothing has been stopped"), "got {text}");
        assert!(text.contains("mtime, not on content"), "got {text}");
        // And the third, which is about the badge's ABSENCE: the detector
        // can decline to answer, and a silent dashboard is not a clean
        // one. Asserted because a static string nothing reads is exactly
        // how this codebase has shipped a lie before.
        assert!(text.contains("no loop was proved"), "got {text}");
    }

    #[test]
    fn the_help_stays_inside_the_width_the_key_table_already_sets() {
        // `?` pages plain grouped text through the pager with no
        // wrapping engine behind it, so a line wider than the shipped
        // key table is one the pager has to chop. 74 is the widest row
        // that ships (`p  freeze the frame in place …`); this section
        // may reach it and must not pass it.
        for line in pane_help(&registry(true))
            .into_iter()
            .chain(pane_help(&live_registry()))
        {
            assert!(
                line.chars().count() <= 74,
                "{} cells: {line:?}",
                line.chars().count()
            );
        }
    }

    #[test]
    fn load_diagnostics_get_their_own_help_section_only_when_present() {
        // The `?` reference is the diagnostics channel: load-time
        // facts worth telling but not worth failing over. Absent
        // diagnostics add NOTHING — the byte-identity pin below stays
        // the proof for the common case.
        let noisy = registry(false).with_diagnostics(vec![
            "duplicate id \"a\" — refs bind to the first declaration".to_string(),
        ]);
        let lines = pane_help(&noisy);
        assert!(
            lines.contains(&"  diagnostics:".to_string()),
            "the section header appears: {lines:?}"
        );
        assert!(
            lines
                .iter()
                .any(|l| l.contains("duplicate id \"a\"") && l.starts_with("    ")),
            "the diagnostic is listed, indented: {lines:?}"
        );
        let quiet = pane_help(&registry(false));
        assert!(
            !quiet.iter().any(|l| l.contains("diagnostics")),
            "no section when there is nothing to say: {quiet:?}"
        );
    }

    #[test]
    fn the_help_is_unchanged_when_no_pane_has_a_trigger() {
        // Byte-identity for the common case, asserted against the
        // literal shipped block rather than against a recomputation of
        // it: a dashboard with no triggers gains no help text at all,
        // and cannot gain any by a later edit without failing here.
        assert_eq!(
            pane_help(&registry(false)),
            vec![
                String::new(),
                "  panes:".to_string(),
                "    a  every 5s".to_string(),
                "    b  every 5s".to_string(),
            ]
        );
    }
}
