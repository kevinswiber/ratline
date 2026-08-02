//! The parsed dashboard declaration and the ONE path that validates it.
//!
//! The constructor (`dashboard_kdl`) produces a [`DashboardFile`] and
//! nothing else; every token in a dashboard file is parsed exactly
//! once, here, so the grammar walk and the rules can never drift
//! apart.
//!
//! ## Interval resolution
//!
//! | pane `interval` | defaults `interval` | triggers | effective |
//! |---|---|---|---|
//! | a token | any | any | the pane's token |
//! | `"never"` | any | any | none — triggers only |
//! | absent | a token | any | the default's token |
//! | absent | absent | some | none — triggers only |
//! | absent | absent | none | 2s |
//!
//! `"never"` is spelled out HERE and nowhere else: teaching it to
//! `core::duration::parse_interval` would leak into `rat watch -n`.

use anyhow::{Context, anyhow, bail};

use crate::core::box_model::{BorderPreset, Sides, parse_sides};
use crate::core::duration::parse_interval;
use crate::core::registry::{
    LayoutNode, Overflow, PaneBox, PaneWidth, Registry, SourceId, SourceSpec,
};
use crate::core::trigger::parse_trigger;

/// `rat watch --trigger-debounce`'s default, reused verbatim so a pane
/// and a watch behave the same when neither says otherwise.
const DEFAULT_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(250);
/// The shipped `rat watch` default interval.
const DEFAULT_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);

/// The parsed declaration. The constructor produces exactly this; ONE
/// validation path turns it into a [`Registry`].
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DashboardFile {
    /// The whole dashboard's name, one bold line above the composed
    /// panes — a different thing from any pane's own `title`, which
    /// labels one box's border.
    pub title: Option<String>,
    pub gap: Option<usize>,
    pub row_gap: Option<usize>,
    pub defaults: PaneDecl,
    pub panes: Vec<PaneDecl>,
    /// The declared layout, by pane NAME (ids resolve at validation);
    /// absent stacks every pane in declaration order. The top-level
    /// items compose as a column, exactly as the engine's tree does.
    pub layout: Option<Vec<LayoutDecl>>,
}

/// A declared layout node: a pane by name, or a row/column of nested
/// nodes — the same recursive shape the engine's `LayoutNode` has had
/// from day one, reached by name instead of id.
#[derive(Clone, PartialEq, Debug)]
pub enum LayoutDecl {
    Pane(String),
    Row(Vec<LayoutDecl>),
    Column(Vec<LayoutDecl>),
}

impl LayoutDecl {
    /// One-cell rows and columns collapse to their cell, so the two
    /// grammars' different spellings of "a full-width pane" reach the
    /// same declaration.
    pub fn normalized(self) -> LayoutDecl {
        match self {
            LayoutDecl::Pane(name) => LayoutDecl::Pane(name),
            LayoutDecl::Row(cells) | LayoutDecl::Column(cells) if cells.len() == 1 => {
                cells.into_iter().next().expect("one cell").normalized()
            }
            LayoutDecl::Row(cells) => {
                LayoutDecl::Row(cells.into_iter().map(LayoutDecl::normalized).collect())
            }
            LayoutDecl::Column(cells) => {
                LayoutDecl::Column(cells.into_iter().map(LayoutDecl::normalized).collect())
            }
        }
    }
}

/// One pane's declaration (or the `defaults` block), tokens unparsed.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PaneDecl {
    pub id: Option<String>,
    /// Split argv, or one raw script string under `shell`.
    pub command: Option<Vec<String>>,
    pub shell: Option<bool>,
    /// "5s" | "never".
    pub interval: Option<String>,
    pub trigger: Option<Vec<String>>,
    pub trigger_debounce: Option<String>,
    pub height: Option<u16>,
    /// "40" | "2fr" | "auto".
    pub width: Option<String>,
    /// "keep-top" | "keep-bottom".
    pub overflow: Option<String>,
    pub border: Option<String>,
    /// `parse_sides` shorthand.
    pub padding: Option<String>,
    pub title: Option<String>,
    pub chrome: Option<bool>,
    /// The child is long-lived: spawn it once and show its output as it
    /// arrives, rather than waiting for an exit that is not coming.
    pub live: Option<bool>,
}

impl DashboardFile {
    /// The ONE validation path: resolve defaults, parse every token,
    /// check the layout, build the registry. Every error names the
    /// pane and the fix.
    pub fn into_registry(self) -> anyhow::Result<Registry> {
        if self.panes.is_empty() {
            bail!("no panes declared: a dashboard needs at least one pane");
        }
        if self.defaults.id.is_some() {
            bail!("`name` is not a default: give each pane its own name");
        }

        let names = self.pane_names()?;
        let mut sources = Vec::with_capacity(self.panes.len());
        let mut boxes = Vec::with_capacity(self.panes.len());
        for (decl, name) in self.panes.iter().zip(&names) {
            sources.push(resolve_source(decl, &self.defaults, name)?);
            boxes.push(resolve_box(decl, &self.defaults, name)?);
        }
        let layout = resolve_layout(self.layout.as_deref(), &names)?;
        Ok(Registry::panes(
            sources,
            boxes,
            layout,
            self.gap.unwrap_or(0),
            self.row_gap.unwrap_or(0),
        )?
        .with_title(self.title.clone()))
    }

    /// Every pane's name, in declaration order. Names are the file's
    /// identity for a source; `SourceId` is the engine's, and this is
    /// where the two are married.
    fn pane_names(&self) -> anyhow::Result<Vec<String>> {
        let mut names: Vec<String> = Vec::with_capacity(self.panes.len());
        for (index, decl) in self.panes.iter().enumerate() {
            let Some(id) = decl.id.as_deref() else {
                bail!("pane #{}: every pane needs an id", index + 1);
            };
            if names.iter().any(|seen| seen == id) {
                bail!("pane {id:?} is declared twice: pane ids must be unique");
            }
            names.push(id.to_string());
        }
        Ok(names)
    }
}

/// Every error a pane can raise names the pane first — the file may
/// have a dozen, and "invalid overflow" alone does not say which one
/// to edit.
fn at(name: &str) -> String {
    format!("pane {name:?}")
}

fn resolve_source(decl: &PaneDecl, defaults: &PaneDecl, id: &str) -> anyhow::Result<SourceSpec> {
    let shell = decl.shell.or(defaults.shell).unwrap_or(false);
    let live = decl.live.or(defaults.live).unwrap_or(false);
    // A command string is word-split (or kept verbatim) at PARSE time,
    // under the shell mode in force where it was written. A pane that
    // inherits the defaults' command while flipping `shell` would
    // execute a wrongly-shaped argv — fail with the fix instead.
    if decl.command.is_none() && shell != defaults.shell.unwrap_or(false) {
        bail!(
            "{}: inherits `command` from `defaults` but overrides `shell` — \
             the inherited command was read under the defaults' shell mode; \
             declare the pane's own `command`",
            at(id)
        );
    }
    let command = decl
        .command
        .clone()
        .or_else(|| defaults.command.clone())
        .filter(|words| !words.is_empty())
        .ok_or_else(|| anyhow!("{}: needs a `command`", at(id)))?;

    let triggers = decl
        .trigger
        .clone()
        .or_else(|| defaults.trigger.clone())
        .unwrap_or_default()
        .iter()
        .map(|spec| parse_trigger(spec).with_context(|| at(id)))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let token = decl.interval.as_deref().or(defaults.interval.as_deref());
    let interval = match (token, triggers.is_empty()) {
        (Some("never"), _) => None,
        (Some(token), _) => Some(parse_interval(token).with_context(|| at(id))?),
        (None, false) => None,
        (None, true) => Some(DEFAULT_INTERVAL),
    };

    let debounce = match decl
        .trigger_debounce
        .as_deref()
        .or(defaults.trigger_debounce.as_deref())
    {
        Some(token) => parse_interval(token).with_context(|| at(id))?,
        None => DEFAULT_DEBOUNCE,
    };

    Ok(SourceSpec {
        id: id.to_string(),
        // Verbatim: a shell pane's script must never round-trip through
        // split-then-join.
        command,
        shell,
        interval,
        triggers,
        debounce,
        live,
    })
}

fn resolve_box(decl: &PaneDecl, defaults: &PaneDecl, id: &str) -> anyhow::Result<PaneBox> {
    let height = decl.height.or(defaults.height).ok_or_else(|| {
        anyhow!(
            "{}: needs a `height` — declare one on the pane or in `defaults`",
            at(id)
        )
    })?;
    let width = match decl.width.as_deref().or(defaults.width.as_deref()) {
        None | Some("auto") => PaneWidth::Weight(1),
        Some(token) => parse_width(token, id)?,
    };
    // `None` and an explicit "keep-top" are NOT the same arm here, even
    // though they resolve alike for a batch pane. A live pane is read at
    // its tail, so an undeclared overflow defaults to keep-bottom; and
    // keeping the head on one silently kills the `D` and `c` change
    // marks, so declaring it is an error rather than a preference.
    let live = decl.live.or(defaults.live).unwrap_or(false);
    let declared_overflow = decl.overflow.as_deref().or(defaults.overflow.as_deref());
    let overflow = match (declared_overflow, live) {
        (None, true) => Overflow::KeepBottom,
        (None, false) => Overflow::KeepTop,
        (Some("keep-top"), true) => bail!(
            "{}: `overflow \"keep-top\"` cannot be combined with `live`: a live \
             pane is read at its tail, and keeping the head silently disables the \
             `D` and `c` change markers. Use `overflow \"keep-bottom\"`, or drop \
             `live`.",
            at(id)
        ),
        (Some("keep-top"), false) => Overflow::KeepTop,
        (Some("keep-bottom"), _) => Overflow::KeepBottom,
        (Some(other), _) => bail!(
            "{}: unknown overflow {other:?}: expected keep-top or keep-bottom",
            at(id)
        ),
    };
    let border = match decl.border.as_deref().or(defaults.border.as_deref()) {
        None => BorderPreset::None,
        Some(token) => parse_border(token, id)?,
    };
    let padding = match decl.padding.as_deref().or(defaults.padding.as_deref()) {
        None => Sides::default(),
        Some(token) => parse_sides(token).with_context(|| at(id))?,
    };
    Ok(PaneBox {
        height,
        width,
        overflow,
        border,
        padding,
        title: decl.title.clone().or_else(|| defaults.title.clone()),
        chrome: decl.chrome.or(defaults.chrome).unwrap_or(true),
    })
}

/// `"40"` = exact cells, `"2fr"` = a share of what is left, `"auto"` =
/// one share. The grammar is CSS-grid-shaped because that is the one
/// every reader already knows.
fn parse_width(token: &str, id: &str) -> anyhow::Result<PaneWidth> {
    let teach = || {
        anyhow!(
            "{}: invalid width {token:?}: expected CELLS, Nfr, or auto",
            at(id)
        )
    };
    if let Some(weight) = token.strip_suffix("fr") {
        return weight
            .trim()
            .parse()
            .map(PaneWidth::Weight)
            .map_err(|_| teach());
    }
    token.parse().map(PaneWidth::Cells).map_err(|_| teach())
}

/// `BorderPreset` has no free-function parser — it is a
/// `clap::ValueEnum`, so the flag and the file accept exactly the same
/// words, and the teaching error enumerates them from the same source.
fn parse_border(token: &str, id: &str) -> anyhow::Result<BorderPreset> {
    use clap::ValueEnum;
    BorderPreset::from_str(token, true).map_err(|_| {
        let known: Vec<String> = BorderPreset::value_variants()
            .iter()
            .filter_map(|preset| preset.to_possible_value().map(|v| v.get_name().to_string()))
            .collect();
        anyhow!(
            "{}: unknown border {token:?}: expected one of {}",
            at(id),
            known.join(", ")
        )
    })
}

/// The declared tree becomes the engine's tree: names resolve to ids,
/// and every declared pane must be placed exactly once at ANY depth —
/// an unplaced pane would run a child nobody can see; a doubly-placed
/// one would have two geometries and one output.
fn resolve_layout(items: Option<&[LayoutDecl]>, names: &[String]) -> anyhow::Result<LayoutNode> {
    let Some(items) = items else {
        return Ok(LayoutNode::Column(
            (0..names.len())
                .map(|i| LayoutNode::Pane(SourceId(i)))
                .collect(),
        ));
    };
    let mut placed = vec![false; names.len()];
    let nodes = items
        .iter()
        .map(|item| resolve_node(item, names, &mut placed))
        .collect::<anyhow::Result<Vec<_>>>()?;
    if nodes.is_empty() {
        bail!("layout is empty: name at least one pane, or omit the layout");
    }
    if let Some(index) = placed.iter().position(|seen| !seen) {
        bail!(
            "pane {:?} is declared but never placed: add it to the layout",
            names[index]
        );
    }
    Ok(LayoutNode::Column(nodes))
}

fn resolve_node(
    decl: &LayoutDecl,
    names: &[String],
    placed: &mut [bool],
) -> anyhow::Result<LayoutNode> {
    match decl {
        LayoutDecl::Pane(wanted) => {
            let id = names
                .iter()
                .position(|name| name == wanted)
                .map(SourceId)
                .ok_or_else(|| {
                    anyhow!(
                        "layout names unknown pane {wanted:?}: declared panes are {}",
                        names.join(", ")
                    )
                })?;
            if std::mem::replace(&mut placed[id.0], true) {
                bail!("layout places pane {wanted:?} twice");
            }
            Ok(LayoutNode::Pane(id))
        }
        LayoutDecl::Row(cells) => {
            if cells.is_empty() {
                bail!("layout has an empty row");
            }
            Ok(LayoutNode::Row(
                cells
                    .iter()
                    .map(|cell| resolve_node(cell, names, placed))
                    .collect::<anyhow::Result<Vec<_>>>()?,
            ))
        }
        LayoutDecl::Column(cells) => {
            if cells.is_empty() {
                bail!("layout has an empty column");
            }
            Ok(LayoutNode::Column(
                cells
                    .iter()
                    .map(|cell| resolve_node(cell, names, placed))
                    .collect::<anyhow::Result<Vec<_>>>()?,
            ))
        }
    }
}

/// Read + parse + validate. `colored` styles the syntax-error
/// snippets for the caller's stream; it changes no parse outcome.
pub fn load(path: &std::path::Path, colored: bool) -> anyhow::Result<Registry> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let file = crate::core::dashboard_kdl::parse_styled(&text, colored)
        .with_context(|| format!("in {}", path.display()))?;
    file.into_registry()
        .with_context(|| format!("in {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::box_model::{BorderPreset, Sides};
    use crate::core::registry::{Composition, LayoutNode, SourceId};

    /// The smallest legal pane: a name, a command, a declared height.
    fn pane(id: &str, command: &[&str]) -> PaneDecl {
        PaneDecl {
            id: Some(id.to_string()),
            command: Some(command.iter().map(|s| s.to_string()).collect()),
            height: Some(3),
            ..PaneDecl::default()
        }
    }

    fn file(panes: Vec<PaneDecl>) -> DashboardFile {
        DashboardFile {
            panes,
            ..DashboardFile::default()
        }
    }

    fn err_of(file: DashboardFile) -> String {
        format!("{:#}", file.into_registry().unwrap_err())
    }

    #[test]
    fn teaching_errors_name_the_kdl_spelling_not_a_toml_section() {
        // `[defaults]` is TOML section syntax, and TOML is gone. The
        // one job of a teaching error is to say what to WRITE; both of
        // these must name the bare `defaults` node.
        let mut heightless = pane("a", &["true"]);
        heightless.height = None;
        let missing_height = err_of(file(vec![heightless]));
        assert!(
            missing_height.contains("`defaults`"),
            "got {missing_height}"
        );
        assert!(
            !missing_height.contains("[defaults]"),
            "got {missing_height}"
        );

        let decl = DashboardFile {
            defaults: PaneDecl {
                command: Some(vec!["true".to_string()]),
                ..PaneDecl::default()
            },
            panes: vec![PaneDecl {
                id: Some("b".to_string()),
                shell: Some(true),
                height: Some(3),
                ..PaneDecl::default()
            }],
            ..DashboardFile::default()
        };
        let shell_override = format!("{:#}", decl.into_registry().unwrap_err());
        assert!(
            shell_override.contains("`defaults`"),
            "got {shell_override}"
        );
        assert!(
            !shell_override.contains("[defaults]"),
            "got {shell_override}"
        );
    }

    #[test]
    fn defaults_fall_through_to_every_pane() {
        let decl = DashboardFile {
            defaults: PaneDecl {
                interval: Some("30s".to_string()),
                border: Some("rounded".to_string()),
                padding: Some("0 1".to_string()),
                height: Some(5),
                ..PaneDecl::default()
            },
            panes: vec![
                PaneDecl {
                    height: Some(4),
                    ..pane("a", &["date"])
                },
                PaneDecl {
                    height: Some(4),
                    ..pane("b", &["date"])
                },
            ],
            ..DashboardFile::default()
        };
        // Both panes declare their own height (4 — the smallest a
        // rounded border plus the chrome row admits), so only the
        // fields the panes leave open come from `defaults`.
        let registry = decl.into_registry().expect("registry");
        assert_eq!(registry.len(), 2);
        for id in registry.ids() {
            assert_eq!(registry.spec(id).interval, Some(Duration::from_secs(30)));
            let box_ = registry.pane(id).expect("a declared pane");
            assert_eq!(box_.border, BorderPreset::Rounded);
            assert_eq!(
                box_.padding,
                Sides {
                    top: 0,
                    right: 1,
                    bottom: 0,
                    left: 1
                }
            );
        }
    }

    #[test]
    fn a_pane_overrides_a_default() {
        let decl = DashboardFile {
            defaults: PaneDecl {
                interval: Some("30s".to_string()),
                height: Some(5),
                ..PaneDecl::default()
            },
            panes: vec![
                PaneDecl {
                    interval: Some("2s".to_string()),
                    height: Some(9),
                    ..pane("fast", &["date"])
                },
                pane("slow", &["date"]),
            ],
            ..DashboardFile::default()
        };
        let registry = decl.into_registry().expect("registry");
        assert_eq!(
            registry.spec(SourceId(0)).interval,
            Some(Duration::from_secs(2))
        );
        assert_eq!(registry.pane(SourceId(0)).expect("pane").height, 9);
        // The pane that said nothing still gets the defaults.
        assert_eq!(
            registry.spec(SourceId(1)).interval,
            Some(Duration::from_secs(30))
        );
        assert_eq!(registry.pane(SourceId(1)).expect("pane").height, 3);
    }

    #[test]
    fn interval_never_means_no_deadline() {
        // "never" is resolved HERE and nowhere else: parse_interval's
        // grammar stays exactly as shipped, or `rat watch -n never` would
        // silently become a legal flag.
        let decl = file(vec![PaneDecl {
            interval: Some("never".to_string()),
            ..pane("manual", &["date"])
        }]);
        let registry = decl.into_registry().expect("registry");
        assert_eq!(registry.spec(SourceId(0)).interval, None);
        assert!(crate::core::duration::parse_interval("never").is_err());
    }

    #[test]
    fn no_interval_and_no_trigger_defaults_to_two_seconds() {
        let registry = file(vec![pane("plain", &["date"])])
            .into_registry()
            .expect("registry");
        assert_eq!(
            registry.spec(SourceId(0)).interval,
            Some(Duration::from_secs(2))
        );
    }

    #[test]
    fn no_interval_with_a_trigger_is_trigger_only() {
        let decl = file(vec![PaneDecl {
            trigger: Some(vec!["file:./state".to_string()]),
            ..pane("watched", &["date"])
        }]);
        let registry = decl.into_registry().expect("registry");
        assert_eq!(registry.spec(SourceId(0)).interval, None);
        assert_eq!(registry.spec(SourceId(0)).triggers.len(), 1);
    }

    #[test]
    fn a_pane_is_not_live_unless_it_says_so() {
        let registry = file(vec![pane("log", &["date"])])
            .into_registry()
            .expect("registry");
        assert!(!registry.spec(SourceId(0)).live);
    }

    #[test]
    fn live_is_carried_onto_the_spec() {
        let registry = file(vec![PaneDecl {
            live: Some(true),
            ..pane("log", &["date"])
        }])
        .into_registry()
        .expect("registry");
        assert!(registry.spec(SourceId(0)).live);
    }

    #[test]
    fn live_is_inheritable_from_defaults_and_overridable_per_pane() {
        let mut decl = file(vec![
            pane("a", &["date"]),
            PaneDecl {
                live: Some(false),
                ..pane("b", &["date"])
            },
        ]);
        decl.defaults.live = Some(true);
        let registry = decl.into_registry().expect("registry");
        assert!(registry.spec(SourceId(0)).live, "inherits the default");
        assert!(
            !registry.spec(SourceId(1)).live,
            "a pane must be able to opt back out"
        );
    }

    #[test]
    fn a_live_pane_with_no_overflow_declared_resolves_to_keep_bottom() {
        // A live pane is read at its TAIL. The shipped `None => KeepTop`
        // default would otherwise hand every live pane the one overflow
        // refused below, making a bare `live` a load error.
        let registry = file(vec![PaneDecl {
            live: Some(true),
            ..pane("log", &["date"])
        }])
        .into_registry()
        .expect("registry");
        assert_eq!(
            registry.pane(SourceId(0)).expect("pane").overflow,
            Overflow::KeepBottom
        );
    }

    #[test]
    fn a_batch_pane_with_no_overflow_declared_still_resolves_to_keep_top() {
        // The shipped default is untouched — a live-only override, not a
        // change to what every other pane does.
        let registry = file(vec![pane("p", &["date"])])
            .into_registry()
            .expect("registry");
        assert_eq!(
            registry.pane(SourceId(0)).expect("pane").overflow,
            Overflow::KeepTop
        );
    }

    #[test]
    fn keep_top_declared_on_a_live_pane_is_refused() {
        // Under keep-top the change marks are computed and then
        // structurally discarded — the window never reaches an index at
        // or past the pane's row count — so `D` and `c` do nothing for
        // that pane and nothing says why. Silent death of two features is
        // worse than a load error.
        let err = err_of(file(vec![PaneDecl {
            live: Some(true),
            overflow: Some("keep-top".to_string()),
            ..pane("log", &["date"])
        }]));
        assert!(err.contains("log"), "names the pane: {err}");
        assert!(err.contains("keep-top"), "names what is wrong: {err}");
        assert!(err.contains("keep-bottom"), "names the fix: {err}");
    }

    #[test]
    fn keep_top_inherited_from_defaults_is_refused_on_a_live_pane_too() {
        // The route that would otherwise slip through: nothing on the
        // pane says keep-top, so a check reading only the pane's own
        // token passes it.
        let mut decl = file(vec![PaneDecl {
            live: Some(true),
            ..pane("log", &["date"])
        }]);
        decl.defaults.overflow = Some("keep-top".to_string());
        let err = err_of(decl);
        assert!(err.contains("log"), "{err}");
    }

    #[test]
    fn keep_top_stays_legal_on_a_batch_pane() {
        // The refusal is about the COMBINATION. keep-top is the shipped
        // default and must not become an error.
        let registry = file(vec![PaneDecl {
            overflow: Some("keep-top".to_string()),
            ..pane("p", &["date"])
        }])
        .into_registry()
        .expect("registry");
        assert_eq!(
            registry.pane(SourceId(0)).expect("pane").overflow,
            Overflow::KeepTop
        );
    }

    #[test]
    fn live_with_keep_bottom_declared_is_accepted() {
        let registry = file(vec![PaneDecl {
            live: Some(true),
            overflow: Some("keep-bottom".to_string()),
            ..pane("log", &["date"])
        }])
        .into_registry()
        .expect("registry");
        assert_eq!(
            registry.pane(SourceId(0)).expect("pane").overflow,
            Overflow::KeepBottom
        );
    }

    #[test]
    fn a_duplicate_pane_name_is_rejected() {
        let err = err_of(file(vec![pane("git", &["date"]), pane("git", &["date"])]));
        assert!(err.contains("git"), "{err}");
        assert!(err.contains("twice") || err.contains("duplicate"), "{err}");
    }

    #[test]
    fn an_unknown_overflow_names_the_two_values() {
        let err = err_of(file(vec![PaneDecl {
            overflow: Some("keep-middle".to_string()),
            ..pane("log", &["date"])
        }]));
        assert!(err.contains("log"), "{err}");
        assert!(err.contains("keep-top"), "{err}");
        assert!(err.contains("keep-bottom"), "{err}");
    }

    #[test]
    fn a_bad_trigger_spec_names_the_pane_and_the_schemes() {
        // parse_trigger already teaches the schemes; the declaration
        // layer only has to say WHICH pane wrote it.
        let err = err_of(file(vec![PaneDecl {
            trigger: Some(vec!["/tmp/state.json".to_string()]),
            ..pane("build", &["date"])
        }]));
        assert!(err.contains("build"), "{err}");
        assert!(err.contains("file:"), "{err}");
    }

    #[test]
    fn an_absent_layout_stacks_panes_in_declaration_order() {
        let registry = file(vec![
            pane("top", &["date"]),
            pane("middle", &["date"]),
            pane("bottom", &["date"]),
        ])
        .into_registry()
        .expect("registry");
        let Composition::Panes { layout, .. } = registry.composition() else {
            panic!("a dashboard composes panes");
        };
        assert_eq!(
            layout,
            &LayoutNode::Column(vec![
                LayoutNode::Pane(SourceId(0)),
                LayoutNode::Pane(SourceId(1)),
                LayoutNode::Pane(SourceId(2)),
            ])
        );
    }

    #[test]
    fn a_layout_naming_an_unknown_pane_lists_the_declared_names() {
        let decl = DashboardFile {
            panes: vec![pane("git", &["date"]), pane("clock", &["date"])],
            layout: Some(vec![LayoutDecl::Row(vec![
                LayoutDecl::Pane("git".to_string()),
                LayoutDecl::Pane("clok".to_string()),
            ])]),
            ..DashboardFile::default()
        };
        let err = err_of(decl);
        assert!(err.contains("clok"), "{err}");
        assert!(err.contains("clock"), "{err}");
    }

    #[test]
    fn a_shell_pane_keeps_its_script_verbatim() {
        // The script never round-trips through split-then-join, or its
        // quoting is destroyed. into_registry copies the vec.
        let script = "date +%H:%M | tr -d '\\n'";
        let decl = file(vec![PaneDecl {
            shell: Some(true),
            command: Some(vec![script.to_string()]),
            ..pane("stamp", &["unused"])
        }]);
        let registry = decl.into_registry().expect("registry");
        let spec = registry.spec(SourceId(0));
        assert!(spec.shell);
        assert_eq!(spec.command, vec![script.to_string()]);
    }

    #[test]
    fn an_inherited_command_with_an_overridden_shell_is_rejected() {
        // The defaults' command string was word-split (or kept verbatim)
        // under the DEFAULTS' shell mode at parse time; a pane that
        // flips `shell` while inheriting it would execute a
        // wrongly-shaped argv. Both directions error, teaching the fix.
        let decl = DashboardFile {
            defaults: PaneDecl {
                shell: Some(true),
                command: Some(vec!["printf inherited".to_string()]),
                height: Some(3),
                ..PaneDecl::default()
            },
            panes: vec![PaneDecl {
                id: Some("plain".to_string()),
                shell: Some(false),
                ..PaneDecl::default()
            }],
            ..DashboardFile::default()
        };
        let err = format!("{:#}", decl.into_registry().unwrap_err());
        assert!(err.contains("plain"), "{err}");
        assert!(err.contains("shell"), "{err}");
        assert!(err.contains("command"), "{err}");

        let reverse = DashboardFile {
            defaults: PaneDecl {
                command: Some(vec!["git".to_string(), "status".to_string()]),
                height: Some(3),
                ..PaneDecl::default()
            },
            panes: vec![PaneDecl {
                id: Some("shelly".to_string()),
                shell: Some(true),
                ..PaneDecl::default()
            }],
            ..DashboardFile::default()
        };
        assert!(reverse.into_registry().is_err());

        // Matching modes inherit fine.
        let ok = DashboardFile {
            defaults: PaneDecl {
                command: Some(vec!["date".to_string()]),
                height: Some(3),
                ..PaneDecl::default()
            },
            panes: vec![
                pane("fine", &["unused"]),
                PaneDecl {
                    id: Some("inheriting".to_string()),
                    command: None,
                    ..pane("inheriting", &["unused"])
                },
            ],
            ..DashboardFile::default()
        };
        assert!(ok.into_registry().is_ok());
    }

    #[test]
    fn a_nested_layout_resolves_rows_within_rows() {
        // A row may hold columns and a column rows, to any depth: the
        // engine's tree was recursive from day one; the declaration
        // now reaches it.
        let decl = DashboardFile {
            panes: vec![
                pane("a", &["date"]),
                pane("b", &["date"]),
                pane("c", &["date"]),
            ],
            layout: Some(vec![LayoutDecl::Row(vec![
                LayoutDecl::Column(vec![
                    LayoutDecl::Pane("a".to_string()),
                    LayoutDecl::Pane("b".to_string()),
                ]),
                LayoutDecl::Pane("c".to_string()),
            ])]),
            ..DashboardFile::default()
        };
        let registry = decl.into_registry().expect("registry");
        let Composition::Panes { layout, .. } = registry.composition() else {
            panic!("a dashboard composes panes");
        };
        assert_eq!(
            layout,
            &LayoutNode::Column(vec![LayoutNode::Row(vec![
                LayoutNode::Column(vec![
                    LayoutNode::Pane(SourceId(0)),
                    LayoutNode::Pane(SourceId(1)),
                ]),
                LayoutNode::Pane(SourceId(2)),
            ])])
        );
    }

    #[test]
    fn nested_layout_errors_still_name_the_pane() {
        // The exactly-once and known-name rules hold at any depth.
        let dup = DashboardFile {
            panes: vec![pane("a", &["date"]), pane("b", &["date"])],
            layout: Some(vec![LayoutDecl::Row(vec![
                LayoutDecl::Pane("a".to_string()),
                LayoutDecl::Column(vec![
                    LayoutDecl::Pane("b".to_string()),
                    LayoutDecl::Pane("a".to_string()),
                ]),
            ])]),
            ..DashboardFile::default()
        };
        let err = format!("{:#}", dup.into_registry().unwrap_err());
        assert!(err.contains("\"a\"") && err.contains("twice"), "{err}");

        let empty = DashboardFile {
            panes: vec![pane("a", &["date"])],
            layout: Some(vec![
                LayoutDecl::Pane("a".to_string()),
                LayoutDecl::Column(Vec::new()),
            ]),
            ..DashboardFile::default()
        };
        let err = format!("{:#}", empty.into_registry().unwrap_err());
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn a_pane_without_a_height_names_the_defaults_key() {
        // Every pane's box height is DECLARED. There is no default to
        // fall back on, so the error has to teach where to put one.
        let err = err_of(file(vec![PaneDecl {
            height: None,
            ..pane("sizeless", &["date"])
        }]));
        assert!(err.contains("sizeless"), "{err}");
        assert!(err.contains("height"), "{err}");
        assert!(err.contains("defaults"), "{err}");
    }
}
