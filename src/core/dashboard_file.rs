//! The format-agnostic dashboard declaration and the ONE path that
//! validates it.
//!
//! Both format constructors (`dashboard_toml`, `dashboard_kdl`) produce
//! a [`DashboardFile`] and nothing else; every token in a dashboard
//! file is parsed exactly once, here, so a rule can never differ
//! between the two grammars.
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

// The declaration layer lands before the subcommand that reads it; the
// dashboard subcommand is the first real caller and removes this.
#![allow(dead_code)]

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

/// Which grammar a declaration file is written in.
#[derive(Copy, Clone, PartialEq, Eq, Debug, clap::ValueEnum)]
pub enum DeclFormat {
    Toml,
    Kdl,
}

/// The format-agnostic declaration. Both constructors produce exactly
/// this; ONE validation path turns it into a [`Registry`].
#[derive(Clone, PartialEq, Debug, Default)]
pub struct DashboardFile {
    pub gap: Option<usize>,
    pub row_gap: Option<usize>,
    pub defaults: PaneDecl,
    pub panes: Vec<PaneDecl>,
    /// Rows of pane names; absent stacks every pane in declaration
    /// order.
    pub layout: Option<Vec<Vec<String>>>,
}

/// One pane's declaration (or the `[defaults]` block), tokens unparsed.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct PaneDecl {
    pub name: Option<String>,
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
}

impl DashboardFile {
    /// The ONE validation path: resolve defaults, parse every token,
    /// check the layout, build the registry. Every error names the
    /// pane and the fix.
    pub fn into_registry(self) -> anyhow::Result<Registry> {
        if self.panes.is_empty() {
            bail!("no panes declared: a dashboard needs at least one pane");
        }
        if self.defaults.name.is_some() {
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
        Registry::panes(
            sources,
            boxes,
            layout,
            self.gap.unwrap_or(0),
            self.row_gap.unwrap_or(0),
        )
    }

    /// Every pane's name, in declaration order. Names are the file's
    /// identity for a source; `SourceId` is the engine's, and this is
    /// where the two are married.
    fn pane_names(&self) -> anyhow::Result<Vec<String>> {
        let mut names: Vec<String> = Vec::with_capacity(self.panes.len());
        for (index, decl) in self.panes.iter().enumerate() {
            let Some(name) = decl.name.as_deref() else {
                bail!("pane #{}: every pane needs a `name`", index + 1);
            };
            if names.iter().any(|seen| seen == name) {
                bail!("pane {name:?} is declared twice: pane names must be unique");
            }
            names.push(name.to_string());
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

fn resolve_source(decl: &PaneDecl, defaults: &PaneDecl, name: &str) -> anyhow::Result<SourceSpec> {
    let command = decl
        .command
        .clone()
        .or_else(|| defaults.command.clone())
        .filter(|words| !words.is_empty())
        .ok_or_else(|| anyhow!("{}: needs a `command`", at(name)))?;
    let shell = decl.shell.or(defaults.shell).unwrap_or(false);

    let triggers = decl
        .trigger
        .clone()
        .or_else(|| defaults.trigger.clone())
        .unwrap_or_default()
        .iter()
        .map(|spec| parse_trigger(spec).with_context(|| at(name)))
        .collect::<anyhow::Result<Vec<_>>>()?;

    let token = decl.interval.as_deref().or(defaults.interval.as_deref());
    let interval = match (token, triggers.is_empty()) {
        (Some("never"), _) => None,
        (Some(token), _) => Some(parse_interval(token).with_context(|| at(name))?),
        (None, false) => None,
        (None, true) => Some(DEFAULT_INTERVAL),
    };

    let debounce = match decl
        .trigger_debounce
        .as_deref()
        .or(defaults.trigger_debounce.as_deref())
    {
        Some(token) => parse_interval(token).with_context(|| at(name))?,
        None => DEFAULT_DEBOUNCE,
    };

    Ok(SourceSpec {
        name: name.to_string(),
        // Verbatim: a shell pane's script must never round-trip through
        // split-then-join.
        command,
        shell,
        interval,
        triggers,
        debounce,
    })
}

fn resolve_box(decl: &PaneDecl, defaults: &PaneDecl, name: &str) -> anyhow::Result<PaneBox> {
    let height = decl.height.or(defaults.height).ok_or_else(|| {
        anyhow!(
            "{}: needs a `height` — declare one on the pane or in [defaults]",
            at(name)
        )
    })?;
    let width = match decl.width.as_deref().or(defaults.width.as_deref()) {
        None | Some("auto") => PaneWidth::Weight(1),
        Some(token) => parse_width(token, name)?,
    };
    let overflow = match decl.overflow.as_deref().or(defaults.overflow.as_deref()) {
        None | Some("keep-top") => Overflow::KeepTop,
        Some("keep-bottom") => Overflow::KeepBottom,
        Some(other) => bail!(
            "{}: unknown overflow {other:?}: expected keep-top or keep-bottom",
            at(name)
        ),
    };
    let border = match decl.border.as_deref().or(defaults.border.as_deref()) {
        None => BorderPreset::None,
        Some(token) => parse_border(token, name)?,
    };
    let padding = match decl.padding.as_deref().or(defaults.padding.as_deref()) {
        None => Sides::default(),
        Some(token) => parse_sides(token).with_context(|| at(name))?,
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
fn parse_width(token: &str, name: &str) -> anyhow::Result<PaneWidth> {
    let teach = || {
        anyhow!(
            "{}: invalid width {token:?}: expected CELLS, Nfr, or auto",
            at(name)
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
fn parse_border(token: &str, name: &str) -> anyhow::Result<BorderPreset> {
    use clap::ValueEnum;
    BorderPreset::from_str(token, true).map_err(|_| {
        let known: Vec<String> = BorderPreset::value_variants()
            .iter()
            .filter_map(|preset| preset.to_possible_value().map(|v| v.get_name().to_string()))
            .collect();
        anyhow!(
            "{}: unknown border {token:?}: expected one of {}",
            at(name),
            known.join(", ")
        )
    })
}

/// Rows of pane names become a `Column` of rows. A one-pane row is a
/// bare `Pane`, matching how the file grammars spell it. Every declared
/// pane must be placed exactly once: an unplaced pane would run a child
/// nobody can see.
fn resolve_layout(rows: Option<&[Vec<String>]>, names: &[String]) -> anyhow::Result<LayoutNode> {
    let id_of = |wanted: &str| -> anyhow::Result<SourceId> {
        names
            .iter()
            .position(|name| name == wanted)
            .map(SourceId)
            .ok_or_else(|| {
                anyhow!(
                    "layout names unknown pane {wanted:?}: declared panes are {}",
                    names.join(", ")
                )
            })
    };
    let Some(rows) = rows else {
        return Ok(LayoutNode::Column(
            (0..names.len())
                .map(|i| LayoutNode::Pane(SourceId(i)))
                .collect(),
        ));
    };
    let mut placed = vec![false; names.len()];
    let mut nodes = Vec::with_capacity(rows.len());
    for row in rows {
        let mut cells = Vec::with_capacity(row.len());
        for wanted in row {
            let id = id_of(wanted)?;
            if std::mem::replace(&mut placed[id.0], true) {
                bail!("layout places pane {wanted:?} twice");
            }
            cells.push(LayoutNode::Pane(id));
        }
        nodes.push(match cells.len() {
            0 => bail!("layout has an empty row"),
            1 => cells.remove(0),
            _ => LayoutNode::Row(cells),
        });
    }
    if let Some(index) = placed.iter().position(|seen| !seen) {
        bail!(
            "pane {:?} is declared but never placed: add it to the layout",
            names[index]
        );
    }
    Ok(LayoutNode::Column(nodes))
}

/// Which grammar a file is written in: the `--format` flag when given,
/// otherwise the extension. `load` itself arrives with the two `parse`
/// functions.
pub fn format_for(path: &std::path::Path, flag: Option<DeclFormat>) -> anyhow::Result<DeclFormat> {
    if let Some(format) = flag {
        return Ok(format);
    }
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("toml") => Ok(DeclFormat::Toml),
        Some("kdl") => Ok(DeclFormat::Kdl),
        _ => Err(anyhow!(
            "cannot tell the format of {}: name it .toml or .kdl, or pass --format",
            path.display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::core::box_model::{BorderPreset, Sides};
    use crate::core::registry::{Composition, LayoutNode, SourceId};

    /// The smallest legal pane: a name, a command, a declared height.
    fn pane(name: &str, command: &[&str]) -> PaneDecl {
        PaneDecl {
            name: Some(name.to_string()),
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
        // fields the panes leave open come from [defaults].
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
            layout: Some(vec![vec!["git".to_string(), "clok".to_string()]]),
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
