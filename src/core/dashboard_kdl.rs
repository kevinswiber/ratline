//! The KDL constructor. A `KdlDocument` walk to [`DashboardFile`] —
//! parsing only; every rule lives once, in `into_registry`. KDL v2
//! grammar (`#true` / `#false` for booleans).

use anyhow::{Context, anyhow, bail};

use crate::core::dashboard_file::{DashboardFile, LayoutDecl, PaneDecl};

/// Every node name a pane or `defaults` block accepts, in the order the
/// teaching error lists them.
const PANE_NODES: &[&str] = &[
    "command",
    "shell",
    "interval",
    "trigger",
    "trigger-debounce",
    "height",
    "width",
    "overflow",
    "border",
    "padding",
    "title",
    "chrome",
];

pub fn parse(text: &str) -> anyhow::Result<DashboardFile> {
    let doc: kdl::KdlDocument = text.parse().context("reading the dashboard KDL")?;
    let mut file = DashboardFile::default();
    // Pane blocks are walked AFTER the whole first pass, so a
    // `defaults` node anywhere in the document still supplies the
    // `shell` the command split depends on.
    let mut panes: Vec<(kdl::KdlNode, String)> = Vec::new();
    for node in doc.nodes() {
        match node.name().value() {
            "gap" => file.gap = Some(usize_field(node, "gap")?),
            "row-gap" => file.row_gap = Some(usize_field(node, "row-gap")?),
            "defaults" => file.defaults = pane_block(node, None, false)?,
            "pane" => panes.push((node.clone(), first_string(node)?)),
            "layout" => file.layout = Some(layout_rows(node)?),
            other => {
                bail!("unknown node {other:?}: expected gap, row-gap, defaults, pane, or layout")
            }
        }
    }
    let default_shell = file.defaults.shell.unwrap_or(false);
    for (node, name) in panes {
        file.panes
            .push(pane_block(&node, Some(name), default_shell)?);
    }
    Ok(file)
}

/// One `pane "name" { … }` or `defaults { … }` block. The block's own
/// `shell` is read FIRST because the command split depends on it —
/// `shell` is the one thing the parser resolves against defaults.
fn pane_block(
    node: &kdl::KdlNode,
    name: Option<String>,
    default_shell: bool,
) -> anyhow::Result<PaneDecl> {
    let children = node.children();
    let shell = children
        .and_then(|doc| doc.get("shell"))
        .map(bool_value)
        .transpose()?;
    let label = name.as_deref().unwrap_or("defaults").to_string();
    let mut decl = PaneDecl {
        name,
        shell,
        ..PaneDecl::default()
    };
    let Some(children) = children else {
        return Ok(decl);
    };
    for child in children.nodes() {
        match child.name().value() {
            "command" => {
                let argv = strings(child)?;
                decl.command = Some(match argv.as_slice() {
                    // One word under `shell` stays one word.
                    [script] if shell.unwrap_or(default_shell) => vec![script.clone()],
                    // An unbalanced string is a parse error naming the
                    // pane — never a one-word fallback that survives
                    // to a spawn.
                    [line] => shell_words::split(line).map_err(|err| {
                        anyhow!("pane {label:?}: command has unbalanced quoting ({err})")
                    })?,
                    argv => argv.to_vec(),
                });
            }
            "shell" => {} // read above
            "chrome" => decl.chrome = Some(bool_value(child)?),
            "trigger" => decl.trigger = Some(strings(child)?),
            "height" => {
                decl.height = Some(u16::try_from(first_int(child)?).map_err(|_| {
                    anyhow!("pane {label:?}: height must be a non-negative integer (max 65535)")
                })?);
            }
            "interval" => decl.interval = Some(first_string(child)?),
            "trigger-debounce" => decl.trigger_debounce = Some(first_string(child)?),
            "width" => decl.width = Some(first_string(child)?),
            "overflow" => decl.overflow = Some(first_string(child)?),
            "border" => decl.border = Some(first_string(child)?),
            "padding" => decl.padding = Some(first_string(child)?),
            "title" => decl.title = Some(first_string(child)?),
            other => bail!(
                "unknown node {other:?}: expected one of {}",
                PANE_NODES.join(", ")
            ),
        }
    }
    Ok(decl)
}

/// `layout { row "a"; row "b" "c"; row { column "d" "e" } }` — a
/// node's string arguments are pane leaves, its children are nested
/// row/column blocks, appended after the arguments in source order.
fn layout_rows(node: &kdl::KdlNode) -> anyhow::Result<Vec<LayoutDecl>> {
    let Some(children) = node.children() else {
        return Ok(Vec::new());
    };
    children
        .nodes()
        .iter()
        .map(|child| Ok(layout_node(child)?.normalized()))
        .collect()
}

fn layout_node(node: &kdl::KdlNode) -> anyhow::Result<LayoutDecl> {
    let mut cells: Vec<LayoutDecl> = strings(node)?.into_iter().map(LayoutDecl::Pane).collect();
    if let Some(children) = node.children() {
        for child in children.nodes() {
            cells.push(layout_node(child)?);
        }
    }
    match node.name().value() {
        "row" => Ok(LayoutDecl::Row(cells)),
        "column" => Ok(LayoutDecl::Column(cells)),
        other => Err(anyhow!(
            "unknown node {other:?} in layout: expected row or column"
        )),
    }
}

/// Every positional argument of a node, as strings.
fn strings(node: &kdl::KdlNode) -> anyhow::Result<Vec<String>> {
    node.entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .map(|entry| {
            entry
                .value()
                .as_string()
                .map(str::to_string)
                .ok_or_else(|| anyhow!("{}: expected a string", node.name().value()))
        })
        .collect()
}

fn first_string(node: &kdl::KdlNode) -> anyhow::Result<String> {
    strings(node)?
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("{}: expected a value", node.name().value()))
}

fn first_int(node: &kdl::KdlNode) -> anyhow::Result<i128> {
    node.entries()
        .iter()
        .find(|entry| entry.name().is_none())
        .and_then(|entry| entry.value().as_integer())
        .ok_or_else(|| anyhow!("{}: expected an integer", node.name().value()))
}

fn bool_value(node: &kdl::KdlNode) -> anyhow::Result<bool> {
    node.entries()
        .iter()
        .find(|entry| entry.name().is_none())
        .and_then(|entry| entry.value().as_bool())
        .ok_or_else(|| anyhow!("{}: expected #true or #false", node.name().value()))
}

/// Checked, field-named conversion: a negative value must FAIL LOUDLY,
/// never wrap — `as usize` would turn `gap -1` into a repeat count near
/// usize::MAX, which `" ".repeat(gap)` would try to allocate.
fn usize_field(node: &kdl::KdlNode, name: &str) -> anyhow::Result<usize> {
    usize::try_from(first_int(node)?).map_err(|_| anyhow!("{name} must be a non-negative integer"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const KDL_FIXTURE: &str = r#"
gap 1

defaults {
    interval "5s"
    border "rounded"
    padding "0 1"
    height 7
}

pane "clock" {
    command "date +%H:%M:%S"
    interval "60s"
    trigger "file:./stamp" "file:./notes"
    height 16
    width "2fr"
}

pane "branch" {
    command "git" "branch" "--show-current"
}

pane "notes" {
    command "rat style hello"
    interval "never"
}

layout {
    row "clock"
    row "branch" "notes"
}
"#;

    #[test]
    fn the_fixture_parses_to_the_declared_dashboard() {
        // The thinness proof, one-sided since the TOML grammar's
        // deletion: the parser emits exactly what the file declares —
        // word splitting, defaults, layout shape — with no rule of its
        // own. (Its two-grammar ancestor also asserted TOML equality;
        // that property lost its second side with the format pick.)
        let from_kdl = parse(KDL_FIXTURE).expect("kdl parses");
        assert_eq!(from_kdl.gap, Some(1));
        assert_eq!(from_kdl.panes.len(), 3);
        assert_eq!(
            from_kdl.panes[0].command,
            Some(vec!["date".to_string(), "+%H:%M:%S".to_string()])
        );
        assert_eq!(
            from_kdl.panes[1].command,
            Some(vec![
                "git".to_string(),
                "branch".to_string(),
                "--show-current".to_string()
            ])
        );
        assert_eq!(from_kdl.defaults.height, Some(7));
        use crate::core::dashboard_file::LayoutDecl;
        assert_eq!(
            from_kdl.layout,
            Some(vec![
                LayoutDecl::Pane("clock".to_string()),
                LayoutDecl::Row(vec![
                    LayoutDecl::Pane("branch".to_string()),
                    LayoutDecl::Pane("notes".to_string()),
                ]),
            ])
        );
    }

    #[test]
    fn nested_layout_blocks_reach_the_recursive_declaration() {
        // row/column blocks nest to any depth and land on the same
        // recursive `LayoutDecl` tree the engine has always had.
        let kdl = parse(
            "pane \"a\" {\n    height 3\n    command \"date\"\n}\npane \"b\" {\n    height 3\n    command \"date\"\n}\npane \"c\" {\n    height 3\n    command \"date\"\n}\nlayout {\n    row {\n        column \"a\" \"b\"\n        column \"c\"\n    }\n}\n",
        )
        .expect("kdl parses");
        use crate::core::dashboard_file::LayoutDecl;
        assert_eq!(
            kdl.layout,
            Some(vec![LayoutDecl::Row(vec![
                LayoutDecl::Column(vec![
                    LayoutDecl::Pane("a".to_string()),
                    LayoutDecl::Pane("b".to_string()),
                ]),
                LayoutDecl::Pane("c".to_string()),
            ])])
        );
    }

    #[test]
    fn an_unknown_kdl_node_names_the_accepted_set() {
        let err = format!(
            "{:#}",
            parse("pane \"a\" {\n    comand \"date\"\n    height 3\n}\n").unwrap_err()
        );
        assert!(err.contains("comand"), "{err}");
        assert!(err.contains("command"), "{err}");
        assert!(err.contains("interval"), "{err}");
    }
}
