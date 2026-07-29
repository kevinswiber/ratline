//! The KDL constructor. A `KdlDocument` walk to [`DashboardFile`] —
//! parsing only; every rule lives once, in `into_registry`. KDL v2
//! grammar (`#true` / `#false` for booleans).

use anyhow::{Context, anyhow, bail};

use crate::core::dashboard_file::{DashboardFile, LayoutDecl, PaneDecl};

/// The one function that puts a key's value on the declaration. The
/// variant IS the key's shape: it says what the value looks like, and
/// therefore where it may be written — a KDL property holds exactly one
/// value, so only `List` lacks a property spelling.
enum Set {
    Text(fn(&mut PaneDecl, String)),
    Count(fn(&mut PaneDecl, i128, &Ctx<'_>) -> anyhow::Result<()>),
    Flag(fn(&mut PaneDecl, bool)),
    List(fn(&mut PaneDecl, Vec<String>, &Ctx<'_>) -> anyhow::Result<()>),
}

impl Set {
    /// A list key has no property spelling; every other shape has both.
    fn takes_a_property(&self) -> bool {
        !matches!(self, Set::List(_))
    }
}

/// One key a `pane` or `defaults` block accepts: its name, the example
/// every teaching error shows, and the one function that applies it.
/// Dispatch, property legality and every accepted-set list read THIS —
/// a new key is one row here and nothing else.
struct Key {
    name: &'static str,
    example: &'static str,
    set: Set,
}

impl Key {
    /// KDL writes a property as `key=value`, so the one example serves
    /// both positions. (List keys have no property spelling and never
    /// reach here.)
    fn property_example(&self) -> String {
        self.example.replacen(' ', "=", 1)
    }
}

const PANE_KEYS: &[Key] = &[
    Key {
        name: "command",
        example: r#"command "git" "log""#,
        set: Set::List(set_command),
    },
    Key {
        name: "shell",
        example: "shell #true",
        set: Set::Flag(|d, v| d.shell = Some(v)),
    },
    Key {
        name: "interval",
        example: r#"interval "5s""#,
        set: Set::Text(|d, v| d.interval = Some(v)),
    },
    Key {
        name: "trigger",
        example: r#"trigger "file:./stamp""#,
        set: Set::List(|d, v, _| {
            d.trigger = Some(v);
            Ok(())
        }),
    },
    Key {
        name: "trigger-debounce",
        example: r#"trigger-debounce "250ms""#,
        set: Set::Text(|d, v| d.trigger_debounce = Some(v)),
    },
    Key {
        name: "height",
        example: "height 7",
        set: Set::Count(set_height),
    },
    Key {
        name: "width",
        example: r#"width "2fr""#,
        set: Set::Text(|d, v| d.width = Some(v)),
    },
    Key {
        name: "overflow",
        example: r#"overflow "keep-bottom""#,
        set: Set::Text(|d, v| d.overflow = Some(v)),
    },
    Key {
        name: "border",
        example: r#"border "rounded""#,
        set: Set::Text(|d, v| d.border = Some(v)),
    },
    Key {
        name: "padding",
        example: r#"padding "0 1""#,
        set: Set::Text(|d, v| d.padding = Some(v)),
    },
    Key {
        name: "title",
        example: r#"title "Recent commits""#,
        set: Set::Text(|d, v| d.title = Some(v)),
    },
    Key {
        name: "chrome",
        example: "chrome #false",
        set: Set::Flag(|d, v| d.chrome = Some(v)),
    },
];

/// What a setter needs beyond the value: the phrase every teaching
/// error opens with, and the shell mode in force where the command was
/// written — the one thing the parser resolves against defaults.
struct Ctx<'a> {
    at: &'a str,
    shell: bool,
}

fn set_command(decl: &mut PaneDecl, argv: Vec<String>, ctx: &Ctx<'_>) -> anyhow::Result<()> {
    decl.command = Some(match argv.as_slice() {
        // One word under `shell` stays one word.
        [script] if ctx.shell => vec![script.clone()],
        // An unbalanced string is a parse error naming the pane — never
        // a one-word fallback that survives to a spawn.
        [line] => shell_words::split(line)
            .map_err(|err| anyhow!("{}: command has unbalanced quoting ({err})", ctx.at))?,
        argv => argv.to_vec(),
    });
    Ok(())
}

fn set_height(decl: &mut PaneDecl, cells: i128, ctx: &Ctx<'_>) -> anyhow::Result<()> {
    decl.height = Some(u16::try_from(cells).map_err(|_| {
        anyhow!(
            "{}: height must be a non-negative integer (max 65535)",
            ctx.at
        )
    })?);
    Ok(())
}

fn key(name: &str) -> Option<&'static Key> {
    PANE_KEYS.iter().find(|k| k.name == name)
}

/// The keys legal in the position the error is complaining about.
fn key_list(property_position: bool) -> String {
    PANE_KEYS
        .iter()
        .filter(|k| !property_position || k.set.takes_a_property())
        .map(|k| k.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Every shape error is generated from the key's own example, so there
/// is no per-key error text to keep in step with the table.
fn shape_err(k: &Key, at: &str) -> anyhow::Error {
    anyhow!(
        "{at}: `{}` takes {} — write `{}`",
        k.name,
        takes(k),
        k.example
    )
}

/// The same complaint against the property spelling, so the fix it
/// shows is the one the user was reaching for.
fn prop_shape_err(k: &Key, at: &str) -> anyhow::Error {
    anyhow!(
        "{at}: `{}` takes {} — write `{}`",
        k.name,
        takes(k),
        k.property_example()
    )
}

fn takes(k: &Key) -> &'static str {
    match k.set {
        Set::Text(_) => "one string",
        Set::Count(_) => "one integer",
        Set::Flag(_) => "#true or #false",
        Set::List(_) => "one or more strings",
    }
}

/// One key, one place, once: a key written twice on the same block —
/// two properties, two child nodes, or one of each — is an error.
/// Last-wins is invisible to a reader scanning a long pane block.
fn record(seen: &mut Vec<&'static str>, k: &'static Key, at: &str) -> anyhow::Result<()> {
    if seen.contains(&k.name) {
        bail!(
            "{at}: `{}` is declared twice — declare it once, as a property or a child node",
            k.name
        );
    }
    seen.push(k.name);
    Ok(())
}

/// The document: settings, then the tree. A pane is declared inside the
/// `row`/`column` that places it.
///
/// The lift is parser-only. A pane's block becomes a `PaneDecl` in
/// `panes` — in document order, which is the order `SourceId`s are
/// handed out — and its name becomes a `LayoutDecl::Pane` in the tree.
/// `DashboardFile` never learns where the panes were written, so
/// `into_registry` stays the one validation path.
pub fn parse(text: &str) -> anyhow::Result<DashboardFile> {
    let doc: kdl::KdlDocument = text.parse().context("reading the dashboard KDL")?;
    let mut file = DashboardFile::default();
    // The tree is walked AFTER the whole first pass, so a `defaults`
    // node anywhere in the document still supplies the `shell` the
    // command split depends on.
    let mut tree: Vec<&kdl::KdlNode> = Vec::new();
    for node in doc.nodes() {
        match node.name().value() {
            "gap" => file.gap = Some(usize_field(node, "gap")?),
            "row-gap" => file.row_gap = Some(usize_field(node, "row-gap")?),
            "defaults" => {
                if !positional(node).is_empty() {
                    bail!("defaults takes no name — it holds the keys every pane inherits");
                }
                file.defaults = pane_block(node, None, false)?;
            }
            "pane" | "row" | "column" => tree.push(node),
            // Placement used to live in its own block, so a reader who
            // writes one is not guessing — they know a grammar that was
            // real. The error owes them the spelling that replaced it.
            "layout" => bail!(
                "there is no `layout` block — a pane is declared inside the row or column \
                 that places it: write `row {{ pane \"log\" {{ … }} pane \"branch\" {{ … }} }}`"
            ),
            other => {
                bail!(
                    "unknown node {other:?} — a dashboard's top level takes \
                     gap, row-gap, defaults, pane, row, or column"
                )
            }
        }
    }
    let default_shell = file.defaults.shell.unwrap_or(false);
    let mut panes = Vec::new();
    let mut items = Vec::with_capacity(tree.len());
    for (index, node) in tree.iter().enumerate() {
        let label = cell_label(None, node, index);
        items.push(inline_node(node, &label, default_shell, &mut panes)?.normalized());
    }
    file.panes = panes;
    // Placement is STRUCTURAL, so the layout is never absent — the top
    // level IS the dashboard's column, and a file of bare panes states
    // that column explicitly (D-2). It resolves to what an absent layout
    // always resolved to.
    file.layout = Some(items);
    Ok(file)
}

/// Where a cell sits, as a breadcrumb — `row #1 > pane #2`. Position is
/// all a container knows about its cells, and often all a teaching error
/// has to point with, since a pane may not have named itself yet.
fn cell_label(inside: Option<&str>, node: &kdl::KdlNode, index: usize) -> String {
    let here = format!("{} #{}", node.name().value(), index + 1);
    match inside {
        Some(path) => format!("{path} > {here}"),
        None => here,
    }
}

/// One node of the inline tree, lifting every pane it meets into
/// `panes`. Its name has already been screened as a cell — by the
/// top-level match in `parse_inline`, or by the container below.
fn inline_node(
    node: &kdl::KdlNode,
    label: &str,
    default_shell: bool,
    panes: &mut Vec<PaneDecl>,
) -> anyhow::Result<LayoutDecl> {
    let kind = node.name().value();
    if kind == "pane" {
        // The same name reader the flat list used: a name is not an
        // internal handle, so exactly one string, unannotated.
        let name = one_name(node, label)?;
        panes.push(pane_block(node, Some(name.clone()), default_shell)?);
        return Ok(LayoutDecl::Pane(name));
    }
    refuse_container_properties(node, label, container_kind(node))?;
    // A bare name here would be the old spelling leaking in: this row's
    // cells ARE the panes, and accepting a name would put one pane's
    // declaration back in two places.
    if !positional(node).is_empty() {
        bail!(
            "{label}: a {kind} holds `pane` blocks, not pane names — \
             declare the pane where it sits, like `{kind} {{ pane \"log\" {{ … }} }}`"
        );
    }
    let cells = node
        .children()
        .map(kdl::KdlDocument::nodes)
        .unwrap_or_default();
    if cells.is_empty() {
        bail!("{label}: this {kind} is empty — put at least one pane in it");
    }
    let mut decls = Vec::with_capacity(cells.len());
    for (index, cell) in cells.iter().enumerate() {
        let inner = cell_label(Some(label), cell, index);
        // What may be a cell is the CONTAINER's rule, so the container
        // is what the error names — `defaults` lands here too, since it
        // is the document's settings and has no position in the geometry.
        let cell_kind = cell.name().value();
        if !matches!(cell_kind, "pane" | "row" | "column") {
            bail!(
                "{inner}: unknown node {cell_kind:?} — {} holds `pane`, `row`, and `column` blocks",
                container_kind(node)
            );
        }
        decls.push(inline_node(cell, &inner, default_shell, panes)?);
    }
    Ok(if kind == "row" {
        LayoutDecl::Row(decls)
    } else {
        LayoutDecl::Column(decls)
    })
}

/// One `pane "name" { … }` or `defaults { … }` block. The block's own
/// `shell` is read FIRST because the command split depends on it —
/// `shell` is the one thing the parser resolves against defaults.
fn pane_block(
    node: &kdl::KdlNode,
    name: Option<String>,
    default_shell: bool,
) -> anyhow::Result<PaneDecl> {
    let at = match name.as_deref() {
        Some(name) => format!("pane {name:?}"),
        None => "defaults".to_string(),
    };
    let shell = peek_shell(node, &at)?;
    let ctx = Ctx {
        at: &at,
        shell: shell.unwrap_or(default_shell),
    };
    let mut decl = PaneDecl {
        name,
        ..PaneDecl::default()
    };
    let mut seen: Vec<&'static str> = Vec::new();

    // Properties first, and NOT behind the children lookup: a braceless
    // `pane "a" height=3` has no children at all.
    for entry in node.entries() {
        let Some(prop) = entry.name() else {
            continue; // positional: the pane's own name
        };
        let prop = prop.value();
        if let Some(ty) = entry.ty() {
            bail!(
                "{at}: the ({}) type annotation on `{prop}` has no meaning here — remove it",
                ty.value()
            );
        }
        let Some(k) = key(prop) else {
            bail!(
                "{at}: unknown property {prop:?} — a pane's keys with a property spelling are {}",
                key_list(true)
            );
        };
        record(&mut seen, k, &at)?;
        match k.set {
            Set::List(_) => bail!(
                "{at}: `{}` holds a list, so it must be a child node — write `{}` inside the block",
                k.name,
                k.example
            ),
            Set::Text(set) => set(&mut decl, prop_text(entry.value(), k, &at)?),
            Set::Count(set) => set(&mut decl, prop_count(entry.value(), k, &at)?, &ctx)?,
            Set::Flag(set) => set(&mut decl, prop_flag(entry.value(), k, &at)?),
        }
    }

    for child in node
        .children()
        .map(kdl::KdlDocument::nodes)
        .unwrap_or_default()
    {
        let name = child.name().value();
        let Some(k) = key(name) else {
            bail!(
                "{at}: unknown node {name:?} — a pane's keys are {}",
                key_list(false)
            );
        };
        record(&mut seen, k, &at)?;
        match k.set {
            Set::Text(set) => set(&mut decl, one_text(child, k, &at)?),
            Set::Count(set) => set(&mut decl, one_count(child, k, &at)?, &ctx)?,
            Set::Flag(set) => set(&mut decl, one_flag(child, k, &at)?),
            Set::List(set) => set(&mut decl, many_text(child, k, &at)?, &ctx)?,
        }
    }
    Ok(decl)
}

/// `shell` is read before the pass that assigns it, because the command
/// split depends on it. A peek only: if `shell` is written in both
/// positions the pass raises the duplicate error, so the peek's choice
/// never reaches a spawn.
fn peek_shell(node: &kdl::KdlNode, at: &str) -> anyhow::Result<Option<bool>> {
    let k = key("shell").expect("`shell` is a pane key");
    if let Some(entry) = node.entry("shell") {
        return prop_flag(entry.value(), k, at).map(Some);
    }
    match node.children().and_then(|doc| doc.get("shell")) {
        Some(child) => one_flag(child, k, at).map(Some),
        None => Ok(None),
    }
}

fn prop_text(value: &kdl::KdlValue, k: &Key, at: &str) -> anyhow::Result<String> {
    value
        .as_string()
        .map(str::to_string)
        .ok_or_else(|| prop_shape_err(k, at))
}

fn prop_count(value: &kdl::KdlValue, k: &Key, at: &str) -> anyhow::Result<i128> {
    value.as_integer().ok_or_else(|| prop_shape_err(k, at))
}

fn prop_flag(value: &kdl::KdlValue, k: &Key, at: &str) -> anyhow::Result<bool> {
    value.as_bool().ok_or_else(|| prop_shape_err(k, at))
}

/// Every positional entry of a node — the values a key was written with.
fn positional(node: &kdl::KdlNode) -> Vec<&kdl::KdlValue> {
    node.entries()
        .iter()
        .filter(|entry| entry.name().is_none())
        .map(kdl::KdlEntry::value)
        .collect()
}

fn one_text(node: &kdl::KdlNode, k: &Key, at: &str) -> anyhow::Result<String> {
    match positional(node).as_slice() {
        [value] => value
            .as_string()
            .map(str::to_string)
            .ok_or_else(|| shape_err(k, at)),
        _ => Err(shape_err(k, at)),
    }
}

fn one_count(node: &kdl::KdlNode, k: &Key, at: &str) -> anyhow::Result<i128> {
    match positional(node).as_slice() {
        [value] => value.as_integer().ok_or_else(|| shape_err(k, at)),
        _ => Err(shape_err(k, at)),
    }
}

fn one_flag(node: &kdl::KdlNode, k: &Key, at: &str) -> anyhow::Result<bool> {
    match positional(node).as_slice() {
        [value] => value.as_bool().ok_or_else(|| shape_err(k, at)),
        _ => Err(shape_err(k, at)),
    }
}

fn many_text(node: &kdl::KdlNode, k: &Key, at: &str) -> anyhow::Result<Vec<String>> {
    let values = positional(node);
    if values.is_empty() {
        return Err(shape_err(k, at));
    }
    values
        .into_iter()
        .map(|value| {
            value
                .as_string()
                .map(str::to_string)
                .ok_or_else(|| shape_err(k, at))
        })
        .collect()
}

/// The container's own name — `a row` / `a column` — for the errors that
/// say what it holds.
fn container_kind(node: &kdl::KdlNode) -> &'static str {
    match node.name().value() {
        "column" => "a column",
        _ => "a row",
    }
}

/// A container holds cells, never keys. `gap` gets its own answer
/// because asking a row for a gap is a reasonable thing to try, and a
/// per-row gap is a decision nobody has made yet.
fn refuse_container_properties(node: &kdl::KdlNode, label: &str, kind: &str) -> anyhow::Result<()> {
    let Some(entry) = node.entries().iter().find(|entry| entry.name().is_some()) else {
        return Ok(());
    };
    let prop = entry.name().expect("filtered to properties").value();
    if prop == "gap" || prop == "row-gap" {
        bail!(
            "{label}: {kind} takes no properties — `{prop}` is the whole dashboard's, declared once at the top level as `{prop} 1`"
        );
    }
    bail!(
        "{label}: {kind} takes no properties, but {prop:?} is set — {kind} holds only `pane`, `row`, and `column` blocks"
    )
}

/// A user's token, quoted the way the rest of the catalog quotes them.
/// KDL v2 lets a string be written bare, and `KdlValue`'s own rendering
/// takes that shortest form — which turns `pane "a" "b"` into an error
/// about `b`, a token the file does not contain.
fn as_written(value: &kdl::KdlValue) -> String {
    match value.as_string() {
        Some(text) => format!("{text:?}"),
        None => value.to_string().trim().to_string(),
    }
}

/// A pane's name: exactly one string, unannotated. It is not an
/// internal handle — it renders as the box title and reaches the child
/// as `RAT_PANE` — so a second name or a number cannot be dropped.
fn one_name(node: &kdl::KdlNode, label: &str) -> anyhow::Result<String> {
    if let Some(ty) = node.ty() {
        bail!(
            "{label}: the ({}) type annotation on a pane has no meaning here — remove it",
            ty.value()
        );
    }
    let values = positional(node);
    match values.as_slice() {
        [value] => value.as_string().map(str::to_string).ok_or_else(|| {
            anyhow!("{label}: a pane's name is a string — write `pane \"log\" {{ … }}`")
        }),
        [first, second, ..] => bail!(
            "{label}: a pane takes ONE name, but {} follows {}",
            as_written(second),
            as_written(first)
        ),
        [] => bail!("{label}: this pane needs a name — write `pane \"log\" {{ … }}`"),
    }
}

/// Checked, field-named conversion: a negative value must FAIL LOUDLY,
/// never wrap — `as usize` would turn `gap -1` into a repeat count near
/// usize::MAX, which `" ".repeat(gap)` would try to allocate.
fn usize_field(node: &kdl::KdlNode, name: &str) -> anyhow::Result<usize> {
    // A document setting's own two answers: it is a node, not a key, so
    // it has no property spelling to offer.
    if node.entries().iter().any(|entry| entry.name().is_some()) {
        bail!("{name} takes no properties — write `{name} 1`");
    }
    let cells = match positional(node).as_slice() {
        [value] => value
            .as_integer()
            .ok_or_else(|| anyhow!("{name} takes one integer — write `{name} 1`"))?,
        _ => bail!("{name} takes one integer — write `{name} 1`"),
    };
    usize::try_from(cells).map_err(|_| anyhow!("{name} must be a non-negative integer"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::registry::Registry;

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

row {
    pane "branch" {
        command "git" "branch" "--show-current"
    }
    pane "notes" {
        command "rat style hello"
        interval "never"
    }
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

    // ---------------------------------------------------------------
    // The inline tree: a pane is declared inside the row or column that
    // places it.
    // ---------------------------------------------------------------

    const INLINE_THREE_PANE: &str = r#"
gap 1

defaults {
    interval "5s"
    border "rounded"
    padding "0 1"
    height 7
}

row {
    pane "log" {
        command "git" "log" "--oneline" "-3"
        interval "15s"
    }
    pane "branch" {
        command "git" "status" "--short" "--branch"
    }
}

row {
    pane "clock" {
        command "date" "+%H:%M:%S"
        interval "1s"
        height 4
    }
}
"#;

    const INLINE_NESTED: &str = r#"
gap 1

defaults {
    interval "5s"
    height 7
}

row {
    column {
        pane "log" {
            command "git" "log" "--oneline" "-3"
            interval "15s"
        }
        pane "branch" {
            command "git" "status" "--short" "--branch"
        }
    }
    column {
        pane "clock" {
            command "date" "+%H:%M:%S"
            interval "1s"
            height 4
        }
    }
}

pane "nested" {
    command "rat" "dashboard" "examples/panes.kdl" "--once"
    height 15
}
"#;

    /// The examples are the grammar most people will read first, so
    /// `include_str!` puts them in the suite: one that stops parsing
    /// fails a test instead of rotting quietly in the repository. (This
    /// is what survives of 3.1's characterization pins — they compared
    /// each example against the text it replaced, and that comparison
    /// lost its second side when the old parser went.)
    #[test]
    fn the_shipped_examples_declare_real_dashboards() {
        for text in [
            include_str!("../../examples/panes.kdl"),
            include_str!("../../examples/panes-nested.kdl"),
        ] {
            parse(text)
                .expect("the example parses")
                .into_registry()
                .expect("the example validates");
        }
    }

    fn names_of(file: &DashboardFile) -> Vec<&str> {
        file.panes
            .iter()
            .map(|decl| decl.name.as_deref().expect("a named pane"))
            .collect()
    }

    /// `Registry` carries no `PartialEq` — it is compared the way the
    /// loop reads it: one source and one box per id, and the tree.
    fn assert_same_registry(left: &Registry, right: &Registry) {
        assert_eq!(left.len(), right.len());
        assert_eq!(left.composition(), right.composition());
        for id in left.ids() {
            assert_eq!(left.spec(id), right.spec(id));
            assert_eq!(left.pane(id), right.pane(id));
        }
    }

    #[test]
    fn an_inline_pane_declares_where_it_sits() {
        let inline = parse(INLINE_THREE_PANE).expect("the inline spelling parses");
        // The lift is visibly a MOVE, not a resolution: a pane written
        // inside its row lands in the flat declaration list, its name
        // lands in the tree, and every token is still exactly what the
        // file wrote.
        assert_eq!(names_of(&inline), ["log", "branch", "clock"]);
        assert_eq!(
            inline.panes[0].command,
            Some(vec![
                "git".to_string(),
                "log".to_string(),
                "--oneline".to_string(),
                "-3".to_string(),
            ])
        );
        assert_eq!(inline.panes[0].interval.as_deref(), Some("15s"));
        assert_eq!(inline.panes[2].height, Some(4));
        assert_eq!(inline.defaults.height, Some(7));
        assert_eq!(inline.gap, Some(1));
        assert_eq!(
            inline.layout,
            Some(vec![
                LayoutDecl::Row(vec![
                    LayoutDecl::Pane("log".to_string()),
                    LayoutDecl::Pane("branch".to_string()),
                ]),
                // The one-cell row collapses to its cell.
                LayoutDecl::Pane("clock".to_string()),
            ])
        );
    }

    #[test]
    fn the_inline_tree_nests_to_the_same_depth() {
        let inline = parse(INLINE_NESTED).expect("the inline spelling parses");
        // Document order is declaration order is `SourceId` order: the
        // walk is depth-first, so a pane's id is its reading position —
        // and the last pane here is a top-level one, after two levels of
        // nesting (D-2).
        assert_eq!(names_of(&inline), ["log", "branch", "clock", "nested"]);
        assert_eq!(inline.panes[3].height, Some(15));
        assert_eq!(
            inline.layout,
            Some(vec![
                LayoutDecl::Row(vec![
                    LayoutDecl::Column(vec![
                        LayoutDecl::Pane("log".to_string()),
                        LayoutDecl::Pane("branch".to_string()),
                    ]),
                    LayoutDecl::Pane("clock".to_string()),
                ]),
                LayoutDecl::Pane("nested".to_string()),
            ])
        );
    }

    #[test]
    fn a_top_level_pane_is_a_cell_in_the_dashboards_column() {
        // D-2: a pane needs no container to be placed. The top level IS
        // the dashboard's column, so a top-level pane is a full-width
        // cell in it — which is why a layout-less file stays a legal
        // file and the minimal dashboard stays one node.
        let file = parse(
            "pane \"a\" {\n    height 3\n    command \"date\"\n}\npane \"b\" {\n    height 3\n    command \"date\"\n}\npane \"c\" {\n    height 3\n    command \"date\"\n}\n",
        )
        .expect("flat panes parse");
        assert_eq!(
            file.layout,
            Some(vec![
                LayoutDecl::Pane("a".to_string()),
                LayoutDecl::Pane("b".to_string()),
                LayoutDecl::Pane("c".to_string()),
            ])
        );

        // …and that stated column is exactly what an ABSENT layout has
        // always resolved to, so nothing about those files changed.
        let implicit = DashboardFile {
            layout: None,
            ..file.clone()
        };
        assert_same_registry(
            &file.into_registry().expect("the stated column validates"),
            &implicit
                .into_registry()
                .expect("the implicit column validates"),
        );
    }

    /// A cell that is not a cell is located by tree position, because
    /// position is all a container knows about it — and often all the
    /// pane has, since it may not have named itself yet. These assert
    /// the WHOLE message: a teaching error that drifts a clause stops
    /// teaching, and there is no second copy of the text to compare to.
    fn container_err(text: &str) -> String {
        format!("{:#}", parse(text).unwrap_err())
    }

    #[test]
    fn an_inline_pane_needs_a_name_and_the_error_names_its_cell() {
        assert_eq!(
            container_err(
                "row {\n    pane \"log\" {\n        command \"date\"\n    }\n    pane {\n        command \"date\"\n    }\n}\n"
            ),
            "row #1 > pane #2: this pane needs a name — write `pane \"log\" { … }`"
        );
    }

    #[test]
    fn a_pane_takes_one_name() {
        assert_eq!(
            container_err("row {\n    pane \"a\" \"b\" {\n        command \"date\"\n    }\n}\n"),
            "row #1 > pane #1: a pane takes ONE name, but \"b\" follows \"a\""
        );
        assert_eq!(
            container_err("row {\n    pane 3 {\n        command \"date\"\n    }\n}\n"),
            "row #1 > pane #1: a pane's name is a string — write `pane \"log\" { … }`"
        );
    }

    /// With the flat list gone a bare name refers to nothing, so the
    /// error is a migration teacher: it shows the one spelling (D-3).
    #[test]
    fn a_row_holds_pane_blocks_not_pane_names() {
        assert_eq!(
            container_err("row \"log\"\n"),
            "row #1: a row holds `pane` blocks, not pane names — declare the pane where it sits, like `row { pane \"log\" { … } }`"
        );
    }

    #[test]
    fn an_empty_container_says_to_put_a_pane_in_it() {
        assert_eq!(
            container_err("row {\n}\n"),
            "row #1: this row is empty — put at least one pane in it"
        );
        assert_eq!(
            container_err(
                "row {\n    pane \"a\" {\n        command \"date\"\n    }\n    column {\n    }\n}\n"
            ),
            "row #1 > column #2: this column is empty — put at least one pane in it"
        );
    }

    /// D-5: a per-row gap is a real future request, so `row gap=2` gets
    /// its own answer rather than a generic refusal — the feature stays
    /// an open decision instead of becoming a shipped lie.
    #[test]
    fn a_row_takes_no_properties() {
        assert_eq!(
            container_err(
                "row style=\"x\" {\n    pane \"a\" {\n        command \"date\"\n    }\n}\n"
            ),
            "row #1: a row takes no properties, but \"style\" is set — a row holds only `pane`, `row`, and `column` blocks"
        );
        assert_eq!(
            container_err("row gap=2 {\n    pane \"a\" {\n        command \"date\"\n    }\n}\n"),
            "row #1: a row takes no properties — `gap` is the whole dashboard's, declared once at the top level as `gap 1`"
        );
    }

    #[test]
    fn an_unknown_node_in_a_row_names_the_three_it_holds() {
        assert_eq!(
            container_err("row {\n    panel {\n        command \"date\"\n    }\n}\n"),
            "row #1 > panel #1: unknown node \"panel\" — a row holds `pane`, `row`, and `column` blocks"
        );
    }

    /// The deleted spelling teaches (I-57). A `layout` block was the
    /// whole of the old grammar's placement, so meeting one is not an
    /// unknown-token complaint — it is a reader who knows the old
    /// spelling, and the error owes them the new one.
    #[test]
    fn a_layout_block_says_there_is_none() {
        assert_eq!(
            container_err(
                "pane \"log\" {\n    command \"date\"\n}\nlayout {\n    row \"log\"\n}\n"
            ),
            "there is no `layout` block — a pane is declared inside the row or column that places it: write `row { pane \"log\" { … } pane \"branch\" { … } }`"
        );
    }

    #[test]
    fn an_unknown_top_level_node_names_the_six() {
        assert_eq!(
            container_err("panes {\n    pane \"log\" {\n        command \"date\"\n    }\n}\n"),
            "unknown node \"panes\" — a dashboard's top level takes gap, row-gap, defaults, pane, row, or column"
        );
    }

    /// D-1: `defaults` has no position in the geometry, so it lives at
    /// the top level beside the tree — inside a container it is just an
    /// unknown node.
    #[test]
    fn a_defaults_block_belongs_at_the_top_level() {
        assert_eq!(
            container_err("row {\n    defaults {\n        height 3\n    }\n}\n"),
            "row #1 > defaults #1: unknown node \"defaults\" — a row holds `pane`, `row`, and `column` blocks"
        );
    }

    #[test]
    fn a_single_cell_row_collapses_to_its_pane() {
        // Every top-level item is normalized, or a one-cell row would
        // declare a different tree than the pane it holds and every
        // equality above would fail.
        let file = parse("row {\n    pane \"clock\" {\n        command \"date\"\n    }\n}\n")
            .expect("a one-cell row parses");
        assert_eq!(
            file.layout,
            Some(vec![LayoutDecl::Pane("clock".to_string())])
        );
    }

    /// One table, twelve keys: every key a pane accepts must land on
    /// the declaration through it. A key the table forgets shows up
    /// here as a `None` field, not as a silently ignored node.
    #[test]
    fn every_pane_key_reaches_the_declaration_through_one_table() {
        let file = parse(
            r#"
pane "all" {
    command "git" "log"
    shell #false
    interval "5s"
    trigger "file:./stamp" "file:./notes"
    trigger-debounce "250ms"
    height 7
    width "2fr"
    overflow "keep-bottom"
    border "rounded"
    padding "0 1"
    title "Recent commits"
    chrome #false
}
"#,
        )
        .expect("parses");
        let pane = &file.panes[0];
        assert_eq!(pane.name.as_deref(), Some("all"));
        assert_eq!(
            pane.command,
            Some(vec!["git".to_string(), "log".to_string()])
        );
        assert_eq!(pane.shell, Some(false));
        assert_eq!(pane.interval.as_deref(), Some("5s"));
        assert_eq!(
            pane.trigger,
            Some(vec!["file:./stamp".to_string(), "file:./notes".to_string()])
        );
        assert_eq!(pane.trigger_debounce.as_deref(), Some("250ms"));
        assert_eq!(pane.height, Some(7));
        assert_eq!(pane.width.as_deref(), Some("2fr"));
        assert_eq!(pane.overflow.as_deref(), Some("keep-bottom"));
        assert_eq!(pane.border.as_deref(), Some("rounded"));
        assert_eq!(pane.padding.as_deref(), Some("0 1"));
        assert_eq!(pane.title.as_deref(), Some("Recent commits"));
        assert_eq!(pane.chrome, Some(false));
    }

    /// I-52 at the pane's keys: a value of the wrong shape, or too many
    /// of them, is refused rather than half-read.
    #[test]
    fn a_pane_key_takes_exactly_the_values_its_shape_allows() {
        for (text, wanted) in [
            (
                "pane \"log\" {\n    command \"date\"\n    interval \"5s\" \"10s\"\n}\n",
                "one string",
            ),
            (
                "pane \"log\" {\n    command \"date\"\n    height \"7\"\n}\n",
                "one integer",
            ),
            (
                "pane \"log\" {\n    command \"date\"\n    chrome \"yes\"\n}\n",
                "#true or #false",
            ),
            ("pane \"log\" {\n    command\n}\n", "one or more strings"),
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains(wanted), "wanted {wanted:?} in {err}");
        }
    }

    #[test]
    fn gap_takes_one_integer_and_nothing_else() {
        for (text, wanted) in [
            ("gap x=1\n", "no properties"),
            ("gap 1 2\n", "one integer"),
            ("gap \"1\"\n", "one integer"),
            ("gap -1\n", "non-negative"),
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains(wanted), "wanted {wanted:?} in {err}");
        }
    }

    #[test]
    fn defaults_takes_no_name() {
        let err = format!(
            "{:#}",
            parse("defaults \"x\" {\n    height 3\n}\n").unwrap_err()
        );
        assert!(err.contains("no name"), "{err}");
    }

    /// The top-level pane name was read with a first-one-wins helper, so
    /// a second name or a non-string was silently dropped.
    #[test]
    fn a_top_level_pane_takes_exactly_one_string_name() {
        for (text, wanted) in [
            ("pane \"a\" \"b\" {\n    height 3\n}\n", "ONE name"),
            ("pane 3 {\n    height 3\n}\n", "is a string"),
            ("(u8)pane \"a\" {\n    height 3\n}\n", "type annotation"),
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains(wanted), "wanted {wanted:?} in {err}");
        }
    }

    /// Slashdash is the kdl crate's job, and it does it before the walk
    /// reaches us — commenting a key out must keep working.
    #[test]
    fn a_commented_out_key_is_not_a_declaration() {
        let file = parse(
            "pane \"log\" /-interval=\"15s\" {\n    command \"date\"\n    /-command \"old\"\n}\n",
        )
        .expect("parses");
        assert_eq!(file.panes[0].interval, None);
        assert_eq!(file.panes[0].command, Some(vec!["date".to_string()]));
    }

    #[test]
    fn a_kdl_type_annotation_is_refused() {
        let err = format!(
            "{:#}",
            parse("pane \"log\" height=(i64)7 {\n    command \"date\"\n}\n").unwrap_err()
        );
        assert!(err.contains("type annotation"), "{err}");
        assert!(err.contains("(i64)"), "{err}");
    }

    /// The C equivalence proof: a scalar key means the same thing on
    /// the block's own line as inside it. Author's choice, uniformly.
    #[test]
    fn a_scalar_key_may_be_written_as_a_property_or_a_child_node() {
        let as_properties = parse(
            r#"
pane "log" interval="15s" height=7 width="2fr" chrome=#false {
    command "git" "log"
}
"#,
        )
        .expect("properties parse");
        let as_children = parse(
            r#"
pane "log" {
    interval "15s"
    height 7
    width "2fr"
    chrome #false
    command "git" "log"
}
"#,
        )
        .expect("children parse");
        assert_eq!(as_properties, as_children);
        assert_eq!(as_properties.panes[0].interval.as_deref(), Some("15s"));
        assert_eq!(as_properties.panes[0].height, Some(7));
    }

    #[test]
    fn defaults_collapses_to_one_line_of_properties() {
        let one_line =
            parse("defaults interval=\"5s\" border=\"rounded\" padding=\"0 1\" height=7\n")
                .expect("parses");
        let block = parse(
            "defaults {\n    interval \"5s\"\n    border \"rounded\"\n    padding \"0 1\"\n    height 7\n}\n",
        )
        .expect("parses");
        assert_eq!(one_line, block);
        assert_eq!(one_line.defaults.border.as_deref(), Some("rounded"));
    }

    /// A KDL property holds exactly one value, so the two list keys
    /// have no property spelling — and the error says where they go.
    #[test]
    fn a_list_key_as_a_property_says_where_it_belongs() {
        for text in [
            "pane \"log\" command=\"git log\" {\n    height 3\n}\n",
            "pane \"log\" trigger=\"file:./x\" {\n    command \"date\"\n}\n",
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains("holds a list"), "{err}");
            assert!(err.contains("child node"), "{err}");
            assert!(err.contains("inside the block"), "{err}");
        }
    }

    #[test]
    fn an_unknown_property_names_the_keys_that_may_be_properties() {
        let err = format!(
            "{:#}",
            parse("pane \"log\" intervl=\"15s\" {\n    command \"date\"\n}\n").unwrap_err()
        );
        assert!(err.contains("unknown property"), "{err}");
        assert!(err.contains("intervl"), "{err}");
        assert!(err.contains("interval"), "{err}");
        assert!(
            !err.contains("command"),
            "a list key has no property spelling, so it must not be offered: {err}"
        );
    }

    /// One key, one place, once — in any spelling combination. Last-wins
    /// is invisible to a reader scanning a long pane block.
    #[test]
    fn a_key_declared_twice_on_one_pane_is_refused() {
        for text in [
            // twice as a property
            "pane \"log\" interval=\"15s\" interval=\"30s\" {\n    command \"date\"\n}\n",
            // twice as a child node
            "pane \"log\" {\n    command \"date\"\n    interval \"15s\"\n    interval \"30s\"\n}\n",
            // once each
            "pane \"log\" interval=\"15s\" {\n    command \"date\"\n    interval \"30s\"\n}\n",
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains("declared twice"), "{err}");
            assert!(err.contains("interval"), "{err}");
        }
    }

    #[test]
    fn a_property_carries_a_kdl_boolean_not_a_quoted_string() {
        let err = format!(
            "{:#}",
            parse("pane \"log\" chrome=\"false\" {\n    command \"date\"\n}\n").unwrap_err()
        );
        assert!(err.contains("#false"), "{err}");
        let ok = parse("pane \"log\" chrome=#false {\n    command \"date\"\n}\n").expect("parses");
        assert_eq!(ok.panes[0].chrome, Some(false));
    }

    /// `shell` is read before the pass that assigns it, because the
    /// command split depends on it — and the property spelling must
    /// reach that read, not just the field.
    #[test]
    fn a_property_holds_the_same_shell_the_command_split_reads() {
        let file = parse("pane \"x\" shell=#true {\n    command \"date +%H | tr -d x\"\n}\n")
            .expect("parses");
        assert_eq!(file.panes[0].shell, Some(true));
        assert_eq!(
            file.panes[0].command,
            Some(vec!["date +%H | tr -d x".to_string()]),
            "one word under shell stays one word"
        );
    }

    /// The teaching error names the pane it happened in, the token the
    /// user wrote, and the keys they meant — all three read off the one
    /// table. Supersedes `an_unknown_kdl_node_names_the_accepted_set`,
    /// which could not name the pane.
    #[test]
    fn an_unknown_pane_key_names_the_pane_and_the_keys() {
        let err = format!(
            "{:#}",
            parse("pane \"log\" {\n    comand \"date\"\n    height 3\n}\n").unwrap_err()
        );
        assert!(err.contains("log"), "names the pane: {err}");
        assert!(err.contains("comand"), "quotes what was written: {err}");
        assert!(err.contains("command"), "{err}");
        assert!(err.contains("interval"), "{err}");
    }
}
