//! The KDL constructor. A `KdlDocument` walk to [`DashboardFile`] —
//! parsing only; every rule lives once, in `into_registry`. KDL v2
//! grammar (`#true` / `#false` for booleans).
//!
//! # The rule
//!
//! Every key a `pane` or `defaults` block accepts holds exactly one
//! value, so it may be written as a property or as a child node,
//! author's choice. `command`'s argv and `trigger`'s specs hold LISTS
//! and have no property spelling, because a KDL property holds exactly
//! one value. `row`, `column`, `gap` and `row-gap` are not keys —
//! containers hold only cells, and `gap`/`row-gap` are the dashboard's,
//! written once at the top level.
//!
//! It is written here, and in `examples/panes.kdl`'s header, because a
//! rule that is real, uniform and mechanically enforced still reads as
//! arbitrary to someone who has only ever met it as an error message
//! (zellij shipped this exact seam undocumented — their #3629).

use anyhow::{anyhow, bail};

use crate::core::dashboard_file::{DashboardFile, LayoutDecl, PaneDecl};
use crate::core::registry::ShellMode;

/// The one function that puts a key's value on the declaration. The
/// variant IS the key's shape: it says what the value looks like, and
/// therefore where it may be written — a KDL property holds exactly one
/// value, so only `List` lacks a property spelling.
enum Set {
    Text(fn(&mut PaneDecl, String)),
    Count(fn(&mut PaneDecl, i128, &Ctx<'_>) -> anyhow::Result<()>),
    Flag(fn(&mut PaneDecl, bool)),
    List(fn(&mut PaneDecl, Vec<String>, &Ctx<'_>) -> anyhow::Result<()>),
    /// `#true`, `#false`, or one string naming the choice — a switch
    /// and a choice in one key. `shell` is the only key with this
    /// shape, so the payload is its own type; a second such key would
    /// earn a shape-neutral one.
    FlagOrText(fn(&mut PaneDecl, ShellMode)),
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
        set: Set::FlagOrText(|d, v| d.shell = Some(v)),
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
    Key {
        name: "live",
        example: "live #true",
        set: Set::Flag(|d, v| d.live = Some(v)),
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
        Set::FlagOrText(_) => "#true, #false, or one string",
    }
}

/// I-55 at the document level: a dashboard has one gap and one set of
/// defaults, so a second declaration is not a refinement of the first —
/// it is a line the reader will believe and the parser would drop.
fn declared_once(name: &'static str, seen: &mut Vec<&'static str>) -> anyhow::Result<()> {
    if seen.contains(&name) {
        bail!("`{name}` is declared twice — a dashboard declares it once");
    }
    seen.push(name);
    Ok(())
}

/// A KDL type annotation never means anything to this grammar, wherever
/// it is hung — on the key node, or on the value the key holds (D-7).
fn refuse_annotation(ty: Option<&kdl::KdlIdentifier>, key: &str, at: &str) -> anyhow::Result<()> {
    match ty {
        Some(ty) => bail!(
            "{at}: the ({}) type annotation on `{key}` has no meaning here — remove it",
            ty.value()
        ),
        None => Ok(()),
    }
}

/// A key node carries its value and NOTHING else. A property hung on
/// it, a block under it, or an annotation anywhere on it is a token
/// with nowhere to go — the same silent discard I-52 closes one level
/// up, one level down.
fn only_a_value(node: &kdl::KdlNode, k: &Key, at: &str) -> anyhow::Result<()> {
    refuse_annotation(node.ty(), k.name, at)?;
    for entry in node.entries() {
        match entry.name() {
            Some(prop) => bail!(
                "{at}: `{}` takes {}, but {:?} is set — write `{}`",
                k.name,
                takes(k),
                prop.value(),
                k.example
            ),
            None => refuse_annotation(entry.ty(), k.name, at)?,
        }
    }
    // `is_some`, not "has nodes": an EMPTY block is still a block the
    // author wrote, and this very message tells them the key holds
    // none. A block they slashdashed OUT never reaches here — the crate
    // strips it before the walk, which is what keeps commenting-out
    // working (D-8).
    if node.children().is_some() {
        bail!(
            "{at}: `{}` takes {} and holds no block — write `{}`",
            k.name,
            takes(k),
            k.example
        );
    }
    Ok(())
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

/// Place a byte offset into 1-based (line, column). The offset WALKS
/// bytes while the column COUNTS chars: kdl 6.7.1's diagnostic spans
/// are byte-indexed despite their doc comment saying chars — winnow's
/// `LocatingSlice` measures `offset_from` in bytes. A `\r` is not a
/// column (CRLF turns the line at its `\n`), a mid-char offset lands
/// after that char, and an offset past the end clamps to wherever the
/// walk stops — nothing here can panic or index out of bounds.
fn line_column(text: &str, offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for (i, ch) in text.char_indices() {
        if i >= offset {
            break;
        }
        match ch {
            '\n' => {
                line += 1;
                column = 1;
            }
            '\r' => {}
            _ => column += 1,
        }
    }
    (line, column)
}

/// The head line: `line N, column M: <message>` — the greppable form
/// scripts and logs key on. Upstream's message ships verbatim:
/// rewriting it is string surgery against a crate that can reword on
/// a patch bump. Help and the other diagnostics are not here — they
/// live in the miette snippet blocks `syntax_error` renders below,
/// which echo and point into the source at a bounded width (the
/// budgeted echo the original one-line-only rule was waiting for).
fn syntax_error_text(line: usize, column: usize, message: Option<&str>) -> String {
    format!(
        "line {line}, column {column}: {}",
        message.unwrap_or("invalid KDL")
    )
}

/// The placed error for a document that failed to parse: the earliest
/// diagnostic by span offset heads the message — the first failure is
/// the one the author can act on — and EVERY diagnostic then renders
/// its own miette snippet block, pointing into the source the way
/// rustc would, with upstream's help text inside the block. An error
/// with no diagnostics keeps the crate's own sentence verbatim;
/// fabricating a position would point at nothing.
///
/// Rendered per-DIAGNOSTIC rather than through the `KdlError` wrapper:
/// the wrapper's own report leads with its constant "Failed to parse
/// KDL document" sentence and an `Error:` separator per related item —
/// noise that says nothing the blocks do not. Unpinned by design: the
/// block glyphs are miette's to change; ours is the head line and the
/// fact that the source is echoed.
fn syntax_error(text: &str, err: &kdl::KdlError, colored: bool) -> anyhow::Error {
    use std::fmt::Write;
    let Some(first) = err.diagnostics.iter().min_by_key(|d| d.span.offset()) else {
        return anyhow!("{err}");
    };
    let (line, column) = line_column(text, first.span.offset());
    let mut message = syntax_error_text(line, column, first.message.as_deref());
    // The caller's stream decides the theme — never an env sniff here,
    // so `--color` and `NO_COLOR` keep their one authority. Colored is
    // plain 16-color ANSI (rustc's own choice): right at every depth,
    // degrading nowhere. The width is fixed either way: this renders
    // into an anyhow message printed by `rat: {err:#}`, where no
    // terminal re-measure reaches.
    let theme = if colored {
        miette::GraphicalTheme {
            characters: miette::ThemeCharacters::unicode(),
            styles: miette::ThemeStyles::ansi(),
        }
    } else {
        miette::GraphicalTheme::unicode_nocolor()
    };
    let handler = miette::GraphicalReportHandler::new_themed(theme).with_width(80);
    let mut blocks: Vec<&kdl::KdlDiagnostic> = err.diagnostics.iter().collect();
    blocks.sort_by_key(|d| d.span.offset());
    for diagnostic in blocks {
        let mut block = String::new();
        if handler.render_report(&mut block, diagnostic).is_ok() {
            let _ = write!(message, "\n{}", block.trim_end());
        }
    }
    anyhow!("{message}")
}

/// The test suite's spelling: `parse_styled` with the plain theme,
/// so hundreds of error-byte assertions never mention color.
#[cfg(test)]
fn parse(text: &str) -> anyhow::Result<DashboardFile> {
    parse_styled(text, false)
}

/// The document: settings, then the tree. A pane is declared inside the
/// `row`/`column` that places it.
///
/// The lift is parser-only. A pane's block becomes a `PaneDecl` in
/// `panes` — in document order, which is the order `SourceId`s are
/// handed out — and its name becomes a `LayoutDecl::Pane` in the tree.
/// `DashboardFile` never learns where the panes were written, so
/// `into_registry` stays the one validation path.
///
/// `colored` styles the error snippets for the caller's stream: it
/// says the reader has a color-capable terminal (the caller's
/// `ColorProfile` verdict — profile detection already folds in
/// `--color`, `NO_COLOR`, `CI`, and the stream's ttyness). The parse
/// outcome is identical either way.
pub fn parse_styled(text: &str, colored: bool) -> anyhow::Result<DashboardFile> {
    let doc: kdl::KdlDocument = text
        .parse()
        .map_err(|err| syntax_error(text, &err, colored))?;
    let mut file = DashboardFile::default();
    // The tree is walked AFTER the whole first pass, so a `defaults`
    // node anywhere in the document still supplies the `shell` the
    // command split depends on.
    let mut tree: Vec<&kdl::KdlNode> = Vec::new();
    let mut settings: Vec<&'static str> = Vec::new();
    for node in doc.nodes() {
        match node.name().value() {
            "title" => {
                declared_once("title", &mut settings)?;
                file.title = Some(title_field(node)?);
            }
            "gap" => {
                declared_once("gap", &mut settings)?;
                file.gap = Some(usize_field(node, "gap")?);
            }
            "row-gap" => {
                declared_once("row-gap", &mut settings)?;
                file.row_gap = Some(usize_field(node, "row-gap")?);
            }
            "defaults" => {
                declared_once("defaults", &mut settings)?;
                refuse_annotation(node.ty(), "defaults", "defaults")?;
                let values = positional(node);
                if !values.is_empty() {
                    // A bare argument that names a key gets the
                    // spelling the author was reaching for; one that
                    // names a document setting gets sent to the top
                    // level. Only a string naming neither keeps the
                    // positional refusal.
                    if let Some(k) = stray_key(&values) {
                        return Err(stray_key_err("defaults", k, &values));
                    }
                    if let Some(name) = stray_setting(&values) {
                        return Err(stray_setting_err("defaults", name, &values));
                    }
                    bail!("defaults takes no id — it holds the keys every pane inherits");
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
                     title, gap, row-gap, defaults, pane, row, or column"
                )
            }
        }
    }
    // The split cares whether a shell is involved, never which one.
    let default_shell = file
        .defaults
        .shell
        .as_ref()
        .is_some_and(ShellMode::runs_a_shell);
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
        let name = one_id(node, label)?;
        panes.push(pane_block(node, Some(name.clone()), default_shell)?);
        return Ok(LayoutDecl::Pane(name));
    }
    refuse_annotation(node.ty(), kind, label)?;
    refuse_container_properties(node, label, container_kind(node))?;
    // A bare name here would be the old spelling leaking in: this row's
    // cells ARE the panes, and accepting a name would put one pane's
    // declaration back in two places. A bare token that names a
    // setting or a pane key gets a teaching answer first — the
    // setting's, then the key's, since `gap` on a row is the nearer
    // miss.
    let values = positional(node);
    if !values.is_empty() {
        if let Some(name) = stray_setting(&values) {
            return Err(stray_setting_err(label, name, &values));
        }
        if let Some(k) = stray_key(&values) {
            bail!(
                "{label}: `{}` is a pane's key — write it on a `pane` block inside this {kind}",
                k.name
            );
        }
        bail!(
            "{label}: a {kind} holds `pane` blocks, not pane ids — \
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
    id: Option<String>,
    default_shell: bool,
) -> anyhow::Result<PaneDecl> {
    let at = match id.as_deref() {
        Some(name) => format!("pane {name:?}"),
        None => "defaults".to_string(),
    };
    let shell = peek_shell(node, &at)?;
    let ctx = Ctx {
        at: &at,
        shell: shell
            .as_ref()
            .map_or(default_shell, ShellMode::runs_a_shell),
    };
    let mut decl = PaneDecl {
        id,
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
            Set::FlagOrText(set) => set(&mut decl, prop_mode(entry.value(), k, &at)?),
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
        only_a_value(child, k, &at)?;
        match k.set {
            Set::Text(set) => set(&mut decl, one_text(child, k, &at)?),
            Set::Count(set) => set(&mut decl, one_count(child, k, &at)?, &ctx)?,
            Set::Flag(set) => set(&mut decl, one_flag(child, k, &at)?),
            Set::List(set) => set(&mut decl, many_text(child, k, &at)?, &ctx)?,
            Set::FlagOrText(set) => set(&mut decl, one_mode(child, k, &at)?),
        }
    }
    Ok(decl)
}

/// `shell` is read before the pass that assigns it, because the command
/// split depends on it. A peek only: if `shell` is written in both
/// positions the pass raises the duplicate error, so the peek's choice
/// never reaches a spawn.
fn peek_shell(node: &kdl::KdlNode, at: &str) -> anyhow::Result<Option<ShellMode>> {
    let k = key("shell").expect("`shell` is a pane key");
    if let Some(entry) = node.entry("shell") {
        return prop_mode(entry.value(), k, at).map(Some);
    }
    match node.children().and_then(|doc| doc.get("shell")) {
        Some(child) => one_mode(child, k, at).map(Some),
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

/// `#true` is the platform's shell, `#false` no shell at all, and a
/// string names the program. An EMPTY string names nothing: a shape
/// error, never a spawn of `""`.
fn shell_value(value: &kdl::KdlValue) -> Option<ShellMode> {
    if let Some(flag) = value.as_bool() {
        return Some(if flag {
            ShellMode::Platform
        } else {
            ShellMode::Direct
        });
    }
    value
        .as_string()
        .filter(|name| !name.trim().is_empty())
        .map(|name| ShellMode::Named(name.to_string()))
}

fn prop_mode(value: &kdl::KdlValue, k: &Key, at: &str) -> anyhow::Result<ShellMode> {
    shell_value(value).ok_or_else(|| prop_shape_err(k, at))
}

fn one_mode(node: &kdl::KdlNode, k: &Key, at: &str) -> anyhow::Result<ShellMode> {
    match positional(node).as_slice() {
        [value] => shell_value(value).ok_or_else(|| shape_err(k, at)),
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

/// A bare positional that names a pane key — found only when a STRING
/// names one. A bare `#true` or `3` names nothing and keeps the
/// positional error it has today; a recognised name at ANY position
/// counts, because `defaults #true shell` still holds the key the
/// author was reaching for.
fn stray_key(values: &[&kdl::KdlValue]) -> Option<&'static Key> {
    values
        .iter()
        .find_map(|value| value.as_string().and_then(key))
}

/// The same scan for the document settings, which are not pane keys —
/// `gap` and `row-gap` are the whole dashboard's.
fn stray_setting(values: &[&kdl::KdlValue]) -> Option<&'static str> {
    values.iter().find_map(|value| {
        ["gap", "row-gap"]
            .into_iter()
            .find(|name| value.as_string() == Some(name))
    })
}

/// The teaching sentence for a pane key written bare in name position:
/// the property spelling, echoing the author's own value when one
/// follows the key so the shown fix is the line they meant to write,
/// the table's example otherwise. A List key has no property spelling,
/// so it gets the child-node sentence with the table's example — the
/// same one the property path teaches.
fn stray_key_err(at: &str, k: &Key, values: &[&kdl::KdlValue]) -> anyhow::Error {
    if let Set::List(_) = k.set {
        return anyhow!(
            "{at}: `{}` holds a list, so it must be a child node — write `{}` inside the block",
            k.name,
            k.example
        );
    }
    // A taught spelling must be WRITABLE: the author's value is echoed
    // only when it fits the key's shape — `pane "a" shell height` must
    // not teach `shell="height"`.
    let fits = |value: &kdl::KdlValue| match k.set {
        Set::Flag(_) => value.as_bool().is_some(),
        Set::Count(_) => value.as_integer().is_some(),
        Set::Text(_) => value.as_string().is_some(),
        Set::List(_) => false,
        // A value that names ANOTHER key is the next stray, not this
        // key's choice — echoing it would teach `shell="height"`.
        Set::FlagOrText(_) => shell_value(value).is_some_and(|mode| match mode {
            ShellMode::Named(name) => key(&name).is_none(),
            _ => true,
        }),
    };
    let spelling = values
        .iter()
        .position(|value| value.as_string() == Some(k.name))
        .and_then(|i| values.get(i + 1))
        .filter(|value| fits(value))
        .map(|value| format!("{}={}", k.name, as_written(value)))
        .unwrap_or_else(|| k.property_example());
    anyhow!(
        "{at}: `{}` is a key, not an id — write `{spelling}`",
        k.name
    )
}

/// The teaching sentence for a document setting written on a block,
/// echoing the author's own value when one follows the name.
fn stray_setting_err(at: &str, name: &str, values: &[&kdl::KdlValue]) -> anyhow::Error {
    // Same writability rule as the key spelling: a setting takes an
    // integer, so anything else is never echoed into the example.
    let example = values
        .iter()
        .position(|value| value.as_string() == Some(name))
        .and_then(|i| values.get(i + 1))
        .filter(|value| value.as_integer().is_some())
        .map(|value| format!("{name} {}", as_written(value)))
        .unwrap_or_else(|| format!("{name} 1"));
    anyhow!(
        "{at}: `{name}` is the whole dashboard's, declared once at the top level as `{example}`"
    )
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

/// A pane's id: exactly one string, unannotated. It is not an
/// internal handle — it renders as the box title and reaches the child
/// as `RAT_PANE` — so a second name or a number cannot be dropped.
fn one_id(node: &kdl::KdlNode, label: &str) -> anyhow::Result<String> {
    // Either position: `(u8)pane "log"` and `pane (name)"log"` are both
    // a token that reaches nothing (D-7).
    let annotation = std::iter::once(node.ty())
        .chain(
            node.entries()
                .iter()
                .filter(|entry| entry.name().is_none())
                .map(kdl::KdlEntry::ty),
        )
        .flatten()
        .next();
    if let Some(ty) = annotation {
        bail!(
            "{label}: the ({}) type annotation on a pane has no meaning here — remove it",
            ty.value()
        );
    }
    let values = positional(node);
    match values.as_slice() {
        [value] => {
            let id = value.as_string().map(str::to_string).ok_or_else(|| {
                anyhow!("{label}: a pane's id is a string — write `pane \"log\" {{ … }}`")
            })?;
            // RFC 3986 unreserved, one or more: every id is literally
            // a valid URI fragment, so `ref="#id"` never needs
            // percent-encoding. Display text belongs in `title`.
            if id.is_empty()
                || !id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'))
            {
                bail!(
                    "{label}: a pane's id sticks to letters, digits, and - . _ ~ — \
                     display text belongs in `title`"
                );
            }
            Ok(id)
        }
        [first, second, ..] => {
            // The scan starts at index 1: index 0 is the id slot,
            // always. KNOWN LIMIT, kept deliberately: `pane shell
            // #true` (the name omitted AND the bare spelling) keeps
            // the name error, because a stray key at index 0 cannot
            // be told apart from a pane genuinely named "shell" —
            // both defects are real, and the name error surfaces
            // first.
            if let Some(k) = stray_key(&values[1..]) {
                return Err(stray_key_err(label, k, &values[1..]));
            }
            bail!(
                "{label}: a pane takes ONE id, but {} follows {}",
                as_written(second),
                as_written(first)
            )
        }
        [] => bail!("{label}: this pane needs an id — write `pane \"log\" {{ … }}`"),
    }
}

/// Checked, field-named conversion: a negative value must FAIL LOUDLY,
/// never wrap — `as usize` would turn `gap -1` into a repeat count near
/// usize::MAX, which `" ".repeat(gap)` would try to allocate.
/// The `title` setting: an optional text, an optional `ref="#id"`,
/// at least one of the two. Same shape checks as the other document
/// settings — no annotation anywhere, no block — plus the reference
/// rules: a ref is a URI FRAGMENT, so a bare string is refused (the
/// whole non-`#` value space stays reserved for URI-references), and
/// the empty fragment (the document itself, per RFC 3986) is refused.
fn title_field(node: &kdl::KdlNode) -> anyhow::Result<crate::core::dashboard_file::TitleDecl> {
    if let Some(ty) = node.ty() {
        bail!(
            "the ({}) type annotation on `title` has no meaning here — remove it",
            ty.value()
        );
    }
    let mut reference = None;
    for entry in node.entries() {
        if let Some(ty) = entry.ty() {
            bail!(
                "the ({}) type annotation on `title` has no meaning here — remove it",
                ty.value()
            );
        }
        let Some(prop) = entry.name() else { continue };
        if prop.value() != "ref" {
            bail!(
                "title's one property is `ref` — write `title \"Deploy status\"` or `title ref=\"#header\"`"
            );
        }
        let Some(value) = entry.value().as_string() else {
            bail!("title's ref takes one string — write `ref=\"#header\"`");
        };
        let Some(fragment) = value.strip_prefix('#') else {
            bail!("title's ref is a URI fragment — write `ref=\"#header\"`");
        };
        if fragment.is_empty() {
            bail!("title ref \"#\" is the whole document — name a pane id, like `ref=\"#header\"`");
        }
        if reference.replace(fragment.to_string()).is_some() {
            bail!("`title` is declared twice — a dashboard declares it once");
        }
    }
    if node.children().is_some() {
        bail!("title holds no block — write `title \"Deploy status\"` or `title ref=\"#header\"`");
    }
    let text =
        match positional(node).as_slice() {
            [] => None,
            [value] => Some(value.as_string().map(str::to_string).ok_or_else(|| {
                anyhow!("title takes one string — write `title \"Deploy status\"`")
            })?),
            _ => bail!("title takes one string — write `title \"Deploy status\"`"),
        };
    if text.is_none() && reference.is_none() {
        bail!(
            "title takes a text, a ref=\"#id\", or both — write `title \"Deploy status\"` or `title ref=\"#header\"`"
        );
    }
    Ok(crate::core::dashboard_file::TitleDecl { text, reference })
}

fn usize_field(node: &kdl::KdlNode, name: &str) -> anyhow::Result<usize> {
    // A document setting's own answers: it is a node, not a key, so it
    // has no property spelling to offer — and like any key it carries
    // its value and nothing else.
    if let Some(ty) = node.ty() {
        bail!(
            "the ({}) type annotation on `{name}` has no meaning here — remove it",
            ty.value()
        );
    }
    if node.entries().iter().any(|entry| entry.name().is_some()) {
        bail!("{name} takes no properties — write `{name} 1`");
    }
    if let Some(entry) = node
        .entries()
        .iter()
        .find(|entry| entry.name().is_none() && entry.ty().is_some())
    {
        bail!(
            "the ({}) type annotation on `{name}` has no meaning here — remove it",
            entry.ty().expect("filtered to annotated").value()
        );
    }
    if node.children().is_some() {
        bail!("{name} takes one integer and holds no block — write `{name} 1`");
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

    #[test]
    fn line_column_counts_from_one_over_bytes_not_chars() {
        assert_eq!(line_column("a\nbb\n", 0), (1, 1));
        assert_eq!(line_column("a\nbb\n", 2), (2, 1));
        assert_eq!(line_column("a\nbb\n", 4), (2, 3));
        // Multi-byte: 'é' is two bytes, one column. Offset 6 is the
        // space — the sixth CHAR — reached by walking BYTES. This is
        // the case that proves the convention (and that a mid-char
        // offset cannot panic the walk).
        assert_eq!(line_column("héllo x", 6), (1, 6));
        // A mid-char byte offset lands after the char it is inside,
        // and never panics: byte 2 is é's second byte, and é is
        // consumed whole, so the walk stops at column 3.
        assert_eq!(line_column("héllo x", 2), (1, 3));
        // CRLF: the \r is not a column.
        assert_eq!(line_column("ab\r\ncd", 4), (2, 1));
        // Past the end: clamp to one past the last content.
        assert_eq!(line_column("ab\n", 99), (2, 1));
    }

    #[test]
    fn a_bare_key_is_found_only_when_it_names_one() {
        use kdl::KdlValue;
        let shell = KdlValue::String("shell".into());
        let nothing = KdlValue::String("frobnicate".into());
        let not_a_string = KdlValue::Bool(true);
        let gap = KdlValue::String("gap".into());
        assert_eq!(stray_key(&[&shell]).map(|k| k.name), Some("shell"));
        assert!(stray_key(&[&nothing]).is_none());
        assert!(stray_key(&[&not_a_string]).is_none());
        // The SECOND positional counts too: `defaults #true shell`
        // still holds a recognisable key.
        assert_eq!(
            stray_key(&[&not_a_string, &shell]).map(|k| k.name),
            Some("shell")
        );
        assert_eq!(stray_setting(&[&gap]), Some("gap"));
        assert!(stray_setting(&[&shell]).is_none());
    }

    #[test]
    fn a_bare_key_on_defaults_teaches_the_property_spelling() {
        assert_eq!(
            container_err("defaults shell #true\npane \"a\" { command \"true\" height 3 }"),
            "defaults: `shell` is a key, not an id — write `shell=#true`"
        );
        assert_eq!(
            container_err("defaults interval \"5s\"\npane \"a\" { command \"true\" height 3 }"),
            "defaults: `interval` is a key, not an id — write `interval=\"5s\"`"
        );
        // A List key has no property spelling — the child-node
        // sentence, the same one the property path teaches.
        assert_eq!(
            container_err(
                "defaults command \"git\" \"log\"\npane \"a\" { command \"true\" height 3 }"
            ),
            "defaults: `command` holds a list, so it must be a child node — write `command \"git\" \"log\"` inside the block"
        );
        // gap is the dashboard's, not a pane key.
        assert_eq!(
            container_err("defaults gap 1\npane \"a\" { command \"true\" height 3 }"),
            "defaults: `gap` is the whole dashboard's, declared once at the top level as `gap 1`"
        );
    }

    #[test]
    fn a_bare_key_after_a_pane_name_teaches_the_property_spelling() {
        assert_eq!(
            container_err("pane \"a\" shell #true { command \"true\" height 3 }"),
            "pane #1: `shell` is a key, not an id — write `shell=#true`"
        );
        // The breadcrumb survives nesting, and the spelling echoes the
        // author's own value.
        assert_eq!(
            container_err("row {\n    pane \"a\" height 3 { command \"true\" }\n}"),
            "row #1 > pane #1: `height` is a key, not an id — write `height=3`"
        );
        assert_eq!(
            container_err("pane \"a\" command \"git\" \"log\" { height 3 }"),
            "pane #1: `command` holds a list, so it must be a child node — write `command \"git\" \"log\"` inside the block"
        );
    }

    #[test]
    fn an_echoed_value_that_does_not_fit_the_key_falls_back_to_the_example() {
        // A taught spelling must be WRITABLE. When the value after the
        // stray key does not fit the key's shape — here a second key
        // name where a boolean belongs — echoing it would teach
        // `shell="height"`, an error of its own. The table's example
        // is the fallback.
        assert_eq!(
            container_err("pane \"a\" shell height 3 { command \"true\" }"),
            "pane #1: `shell` is a key, not an id — write `shell=#true`"
        );
        // A string that names no other key DOES fit `shell` now, so
        // the author's own shell name is the taught spelling.
        assert_eq!(
            container_err("pane \"a\" shell fish { command \"true\" }"),
            "pane #1: `shell` is a key, not an id — write `shell=\"fish\"`"
        );
        assert_eq!(
            container_err("defaults height #true\npane \"a\" { command \"true\" height 3 }"),
            "defaults: `height` is a key, not an id — write `height=7`"
        );
        // The same rule for a document setting: `gap` takes an
        // integer, so a non-integer is never echoed into the example.
        assert_eq!(
            container_err("row gap #true { pane \"a\" height 3 { command \"true\" } }"),
            "row #1: `gap` is the whole dashboard's, declared once at the top level as `gap 1`"
        );
    }

    #[test]
    fn a_bare_gap_on_a_container_names_the_dashboards_gap() {
        assert_eq!(
            container_err("row gap 1 { pane \"a\" height 3 { command \"true\" } }"),
            "row #1: `gap` is the whole dashboard's, declared once at the top level as `gap 1`"
        );
        // The other container, the other setting, the author's own
        // value — one template, every substitution pinned.
        assert_eq!(
            container_err("column row-gap 2 { pane \"a\" height 3 { command \"true\" } }"),
            "column #1: `row-gap` is the whole dashboard's, declared once at the top level as `row-gap 2`"
        );
    }

    #[test]
    fn a_bare_pane_key_on_a_container_says_where_it_belongs() {
        assert_eq!(
            container_err("row shell #true { pane \"a\" height 3 { command \"true\" } }"),
            "row #1: `shell` is a pane's key — write it on a `pane` block inside this row"
        );
    }

    #[test]
    fn a_colored_parse_paints_the_snippet_and_the_plain_one_does_not() {
        let bad = "pane \"log\" interval=5s {\n    command \"date\"\n}\n";
        let colored = format!("{:#}", parse_styled(bad, true).expect_err("still invalid"));
        let plain = format!("{:#}", parse_styled(bad, false).expect_err("still invalid"));
        assert!(colored.contains('\u{1b}'), "got {colored:?}");
        assert!(
            colored.starts_with("line 1, column "),
            "color never touches the greppable head: {colored:?}"
        );
        assert!(!plain.contains('\u{1b}'), "got {plain:?}");
        assert_eq!(
            plain,
            format!("{:#}", parse(bad).expect_err("still invalid")),
            "`parse` IS the plain spelling"
        );
    }

    #[test]
    fn a_kdl_syntax_error_carries_its_line_and_column() {
        // The reported repro: a bare token in property position is
        // not a KDL value, and the answer must say where — AND show
        // the offending line itself, rustc-style. The snippet's exact
        // glyphs are miette's and stay unpinned; the SOURCE ECHO is
        // the contract.
        let err = parse("pane \"log\" interval=5s {\n    command \"date\"\n}\n")
            .expect_err("5s is not a valid KDL value");
        let text = format!("{err:#}");
        assert!(text.starts_with("line 1, column "), "got {text}");
        assert!(!text.contains("Failed to parse KDL document"), "got {text}");
        assert!(
            text.contains("pane \"log\" interval=5s {"),
            "the offending source line is echoed: {text}"
        );
    }

    #[test]
    fn an_error_with_no_diagnostics_keeps_the_crates_own_sentence() {
        // The fallback is the crate's Display, compared at runtime —
        // upstream's bytes are not ours to freeze.
        let err = kdl::KdlError {
            input: std::sync::Arc::new(String::new()),
            diagnostics: Vec::new(),
        };
        assert_eq!(
            format!("{}", syntax_error("", &err, false)),
            format!("{err}")
        );
    }

    #[test]
    fn the_earliest_diagnostic_heads_and_every_diagnostic_gets_a_block() {
        // Two bad values on two lines: the first failure heads the
        // message, and each diagnostic renders its own snippet block
        // below — no counting, no hiding. The `[line:column]` span
        // marker is the loosest stable probe of miette's block
        // header; revisit if a miette bump reformats it.
        let err = parse("a 1.\nb 2.\n").expect_err("both floats are invalid");
        let text = format!("{err:#}");
        assert!(text.starts_with("line 1, column "), "got {text}");
        assert!(text.contains("[1:3]"), "the first block is placed: {text}");
        assert!(text.contains("[2:3]"), "the second block is placed: {text}");
    }

    #[test]
    fn the_head_line_is_the_place_and_the_message() {
        // OUR frame is pinned whole; upstream's message text is a
        // plain value here, never byte-pinned at the route. Help and
        // the other diagnostics live in the snippet blocks now, so
        // the head is just the greppable place + message.
        assert_eq!(
            syntax_error_text(1, 20, Some("Expected valid value")),
            "line 1, column 20: Expected valid value"
        );
        assert_eq!(
            syntax_error_text(3, 1, Some("No closing '}' for child block")),
            "line 3, column 1: No closing '}' for child block"
        );
        assert_eq!(
            syntax_error_text(1, 1, None),
            "line 1, column 1: invalid KDL"
        );
    }

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
            include_str!("../../examples/follow.kdl"),
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
            .map(|decl| decl.id.as_deref().expect("a named pane"))
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
            "row #1 > pane #2: this pane needs an id — write `pane \"log\" { … }`"
        );
    }

    #[test]
    fn a_pane_takes_one_id() {
        assert_eq!(
            container_err("row {\n    pane \"a\" \"b\" {\n        command \"date\"\n    }\n}\n"),
            "row #1 > pane #1: a pane takes ONE id, but \"b\" follows \"a\""
        );
        assert_eq!(
            container_err("row {\n    pane 3 {\n        command \"date\"\n    }\n}\n"),
            "row #1 > pane #1: a pane's id is a string — write `pane \"log\" { … }`"
        );
    }

    /// With the flat list gone a bare name refers to nothing, so the
    /// error is a migration teacher: it shows the one spelling (D-3).
    #[test]
    fn a_row_holds_pane_blocks_not_pane_names() {
        assert_eq!(
            container_err("row \"log\"\n"),
            "row #1: a row holds `pane` blocks, not pane ids — declare the pane where it sits, like `row { pane \"log\" { … } }`"
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
    fn an_unknown_top_level_node_names_the_seven() {
        assert_eq!(
            container_err("panes {\n    pane \"log\" {\n        command \"date\"\n    }\n}\n"),
            "unknown node \"panes\" — a dashboard's top level takes title, gap, row-gap, defaults, pane, row, or column"
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
        assert_eq!(pane.id.as_deref(), Some("all"));
        assert_eq!(
            pane.command,
            Some(vec!["git".to_string(), "log".to_string()])
        );
        assert_eq!(pane.shell, Some(ShellMode::Direct));
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
            (
                "pane \"log\" {\n    command \"date\"\n    shell 3\n}\n",
                "#true, #false, or one string",
            ),
            ("pane \"log\" {\n    command\n}\n", "one or more strings"),
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains(wanted), "wanted {wanted:?} in {err}");
        }
    }

    #[test]
    fn an_id_sticks_to_unreserved_characters() {
        // RFC 3986 unreserved, one or more: every id is literally a
        // valid URI fragment, so reference syntax never needs
        // percent-encoding. Display text belongs in `title`.
        for bad in ["repo status", "a#b", "a/b", "caf\u{e9}", "a%b", ""] {
            let text = format!("pane {bad:?} {{\n    height 3\n    command \"date\"\n}}\n");
            let err = format!("{:#}", parse(&text).unwrap_err());
            assert!(err.contains("letters, digits"), "for {bad:?}: {err}");
        }
        for good in ["a", "A-1", "a.b", "under_score", "til~de", "0"] {
            let text = format!("pane {good:?} {{\n    height 3\n    command \"date\"\n}}\n");
            parse(&text).unwrap_or_else(|e| panic!("{good:?} should parse: {e:#}"));
        }
    }

    #[test]
    fn a_title_ref_parses_with_and_without_fallback_text() {
        let file = parse(
            "title ref=\"#header\"\npane \"header\" {\n    height 3\n    command \"date\"\n}\n",
        )
        .expect("parses");
        let title = file.title.expect("declared");
        assert_eq!(title.text, None);
        assert_eq!(title.reference.as_deref(), Some("header"));
        let file = parse(
            "title \"Fallback\" ref=\"#header\"\npane \"header\" {\n    height 3\n    command \"date\"\n}\n",
        )
        .expect("parses");
        let title = file.title.expect("declared");
        assert_eq!(title.text.as_deref(), Some("Fallback"));
        assert_eq!(title.reference.as_deref(), Some("header"));
    }

    #[test]
    fn a_title_ref_keeps_the_value_space_reserved() {
        // A bare string is refused so every non-fragment spelling
        // stays open for URI-references later; the empty fragment is
        // the whole document and is refused too.
        for (text, wanted) in [
            ("title ref=\"header\"\n", "write `ref=\"#header\"`"),
            ("title ref=\"#\"\n", "name a pane id"),
            ("title ref=3\n", "one string"),
            ("title bogus=\"x\"\n", "`ref`"),
            ("title\n", "a text, a ref"),
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains(wanted), "wanted {wanted:?} in {err}");
        }
    }

    #[test]
    fn a_dashboard_title_parses_and_both_meanings_coexist() {
        // The same word at two positions carries two meanings: the
        // top-level `title` names the whole dashboard, a pane's
        // `title` labels its own border. Both survive one file.
        let file = parse(
            "title \"Deploy status\"\npane \"build\" {\n    height 3\n    command \"date\"\n    title \"Build log\"\n}\n",
        )
        .expect("parses");
        let declared = file.title.expect("declared");
        assert_eq!(declared.text.as_deref(), Some("Deploy status"));
        assert_eq!(declared.reference, None);
        assert_eq!(file.panes[0].title.as_deref(), Some("Build log"));
        // Undeclared stays absent — the row is not rendered from an
        // empty string.
        let bare = parse("pane \"a\" {\n    height 3\n    command \"date\"\n}\n").expect("parses");
        assert_eq!(bare.title, None);
    }

    #[test]
    fn the_dashboard_title_reaches_the_composition() {
        use crate::core::registry::Composition;
        let registry =
            parse("title \"Deploy status\"\npane \"a\" {\n    height 3\n    command \"date\"\n}\n")
                .expect("parses")
                .into_registry()
                .expect("validates");
        let Composition::Panes { title, .. } = registry.composition() else {
            panic!("a dashboard registry composes panes");
        };
        assert_eq!(
            *title,
            crate::core::registry::TitleSource::Static("Deploy status".to_string())
        );
        let registry = parse("pane \"a\" {\n    height 3\n    command \"date\"\n}\n")
            .expect("parses")
            .into_registry()
            .expect("validates");
        let Composition::Panes { title, .. } = registry.composition() else {
            panic!("a dashboard registry composes panes");
        };
        assert_eq!(*title, crate::core::registry::TitleSource::None);
    }

    #[test]
    fn a_title_ref_binds_the_first_declaration_and_an_unknown_ref_teaches() {
        use crate::core::registry::{Composition, SourceId, TitleSource};
        // First-win, the same rule duplicates follow everywhere.
        let registry = parse(
            "title ref=\"#x\"\npane \"x\" {\n    height 3\n    command \"date\"\n}\npane \"x\" {\n    height 3\n    command \"uptime\"\n}\n",
        )
        .expect("parses")
        .into_registry()
        .expect("validates");
        let Composition::Panes { title, .. } = registry.composition() else {
            panic!("panes")
        };
        assert_eq!(
            *title,
            TitleSource::Pane {
                source: SourceId(0),
                fallback: None
            }
        );
        // An id nothing declares is a load error that lists what exists.
        let err = format!(
            "{:#}",
            parse("title ref=\"#nope\"\npane \"a\" {\n    height 3\n    command \"date\"\n}\n")
                .expect("parses")
                .into_registry()
                .unwrap_err()
        );
        assert!(err.contains("names no pane"), "{err}");
        assert!(err.contains("declared ids are a"), "{err}");
    }

    #[test]
    fn title_takes_one_string_and_nothing_else() {
        for (text, wanted) in [
            ("title x=\"y\"\n", "`ref`"),
            ("title \"a\" \"b\"\n", "one string"),
            ("title 3\n", "one string"),
            ("title \"a\" {\n}\n", "holds no block"),
            ("(u8)title \"a\"\n", "type annotation"),
            ("title (u8)\"a\"\n", "type annotation"),
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
    fn defaults_takes_no_id() {
        let err = format!(
            "{:#}",
            parse("defaults \"x\" {\n    height 3\n}\n").unwrap_err()
        );
        assert!(err.contains("no id"), "{err}");
    }

    /// The top-level pane name was read with a first-one-wins helper, so
    /// a second name or a non-string was silently dropped.
    #[test]
    fn a_top_level_pane_takes_exactly_one_string_name() {
        for (text, wanted) in [
            ("pane \"a\" \"b\" {\n    height 3\n}\n", "ONE id"),
            ("pane 3 {\n    height 3\n}\n", "is a string"),
            ("(u8)pane \"a\" {\n    height 3\n}\n", "type annotation"),
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains(wanted), "wanted {wanted:?} in {err}");
        }
    }

    /// I-52 reaches the key node itself, not just the block holding it.
    /// A key node carries its value and nothing else — a property, a
    /// block, or an annotation hung on it is a token with nowhere to go.
    #[test]
    fn a_key_node_carries_its_value_and_nothing_else() {
        for (tail, wanted) in [
            (
                "interval \"5s\" bogus=\"x\"",
                "pane \"log\": `interval` takes one string, but \"bogus\" is set — write `interval \"5s\"`",
            ),
            (
                "interval \"5s\" { junk \"x\" }",
                "pane \"log\": `interval` takes one string and holds no block — write `interval \"5s\"`",
            ),
            // An EMPTY block is still a block the author wrote. The
            // message says the key holds none, so accepting `{}` would
            // make the parser contradict its own error.
            (
                "interval \"5s\" {}",
                "pane \"log\": `interval` takes one string and holds no block — write `interval \"5s\"`",
            ),
            (
                "(u8)interval \"5s\"",
                "pane \"log\": the (u8) type annotation on `interval` has no meaning here — remove it",
            ),
            (
                "interval (string)\"5s\"",
                "pane \"log\": the (string) type annotation on `interval` has no meaning here — remove it",
            ),
        ] {
            assert_eq!(
                container_err(&format!(
                    "pane \"log\" {{\n    height 3\n    {tail}\n    command \"date\"\n}}\n"
                )),
                wanted
            );
        }
    }

    /// D-7 has no exceptions: an annotation means nothing to this
    /// grammar on ANY node, and a container or a name is a node like the
    /// rest. The key nodes were covered first and these were not, which
    /// is the same hole in a different position.
    #[test]
    fn an_annotation_is_refused_on_a_container_or_a_name() {
        for (text, wanted) in [
            (
                "(u8)row {\n    pane \"a\" { height 3; command \"date\" }\n}\n",
                "row #1: the (u8) type annotation on `row` has no meaning here — remove it",
            ),
            (
                "row {\n    (u8)column { pane \"a\" { height 3; command \"date\" } }\n}\n",
                "row #1 > column #1: the (u8) type annotation on `column` has no meaning here — remove it",
            ),
            (
                "(u8)defaults { height 3 }\npane \"a\" { command \"date\" }\n",
                "defaults: the (u8) type annotation on `defaults` has no meaning here — remove it",
            ),
            (
                "pane (name)\"x\" {\n    height 3\n    command \"date\"\n}\n",
                "pane #1: the (name) type annotation on a pane has no meaning here — remove it",
            ),
            (
                "row {\n    pane (name)\"x\" { height 3; command \"date\" }\n}\n",
                "row #1 > pane #1: the (name) type annotation on a pane has no meaning here — remove it",
            ),
        ] {
            assert_eq!(container_err(text), wanted);
        }
    }

    /// A document setting is a key too: `gap` holds one integer, so a
    /// block or an annotation on it reaches nothing.
    #[test]
    fn a_document_setting_carries_its_value_and_nothing_else() {
        for block in ["{ junk \"x\" }", "{}"] {
            assert_eq!(
                container_err(&format!(
                    "gap 1 {block}\npane \"a\" {{ height 3; command \"date\" }}\n"
                )),
                "gap takes one integer and holds no block — write `gap 1`"
            );
        }
        assert_eq!(
            container_err("(u8)gap 1\npane \"a\" { height 3; command \"date\" }\n"),
            "the (u8) type annotation on `gap` has no meaning here — remove it"
        );
    }

    /// I-55 at the document level: last-wins is as invisible here as it
    /// is inside a pane block, and a second `defaults` silently
    /// replacing the first is the same silent discard in a new place.
    #[test]
    fn a_document_setting_is_declared_once() {
        for (text, wanted) in [
            (
                "gap 1\ngap 5\npane \"a\" { height 3; command \"date\" }\n",
                "`gap` is declared twice — a dashboard declares it once",
            ),
            (
                "row-gap 1\nrow-gap 5\npane \"a\" { height 3; command \"date\" }\n",
                "`row-gap` is declared twice — a dashboard declares it once",
            ),
            (
                "defaults { height 3 }\ndefaults { height 9 }\npane \"a\" { command \"date\" }\n",
                "`defaults` is declared twice — a dashboard declares it once",
            ),
            (
                "title \"a\"\ntitle \"b\"\npane \"a\" { height 3; command \"date\" }\n",
                "`title` is declared twice — a dashboard declares it once",
            ),
        ] {
            assert_eq!(container_err(text), wanted);
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

    /// The boundary the "holds no block" rule must not cross: a block
    /// the author COMMENTED OUT was not written, so refusing it would
    /// punish the ordinary way of temporarily removing something.
    #[test]
    fn a_commented_out_block_is_not_a_block() {
        let file = parse(
            "pane \"log\" {\n    height 3\n    interval \"5s\" /-{ junk \"x\" }\n    command \"date\"\n}\n",
        )
        .expect("a slashdashed block is not a block");
        assert_eq!(file.panes[0].interval.as_deref(), Some("5s"));
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
        assert_eq!(file.panes[0].shell, Some(ShellMode::Platform));
        assert_eq!(
            file.panes[0].command,
            Some(vec!["date +%H | tr -d x".to_string()]),
            "one word under shell stays one word"
        );
        // The split follows runs_a_shell, never WHICH shell: a named
        // shell keeps its script one word exactly as #true does.
        let file = parse("pane \"x\" shell=\"fish\" {\n    command \"date +%H | tr -d x\"\n}\n")
            .expect("parses");
        assert_eq!(
            file.panes[0].shell,
            Some(ShellMode::Named("fish".to_string()))
        );
        assert_eq!(
            file.panes[0].command,
            Some(vec!["date +%H | tr -d x".to_string()]),
            "one word under a named shell stays one word"
        );
    }

    /// `#true` is the platform's shell, `#false` no shell at all, and a
    /// string names the program — in both spellings, both positions.
    #[test]
    fn a_shell_key_names_the_shell_it_runs() {
        for (text, wanted) in [
            (
                "pane \"x\" shell=#true {\n    command \"date\"\n}\n",
                ShellMode::Platform,
            ),
            (
                "pane \"x\" {\n    shell #true\n    command \"date\"\n}\n",
                ShellMode::Platform,
            ),
            (
                "pane \"x\" shell=#false {\n    command \"date\"\n}\n",
                ShellMode::Direct,
            ),
            (
                "pane \"x\" shell=\"fish\" {\n    command \"date\"\n}\n",
                ShellMode::Named("fish".to_string()),
            ),
            (
                "pane \"x\" {\n    shell \"fish\"\n    command \"date\"\n}\n",
                ShellMode::Named("fish".to_string()),
            ),
        ] {
            let file = parse(text).expect("parses");
            assert_eq!(file.panes[0].shell, Some(wanted.clone()), "{text}");
        }
        // An EMPTY string names nothing — a shape error naming the
        // accepted values, never a spawn of `""`.
        for text in [
            "pane \"x\" shell=\"\" {\n    command \"date\"\n}\n",
            "pane \"x\" shell=\"   \" {\n    command \"date\"\n}\n",
            "pane \"x\" {\n    shell \"\"\n    command \"date\"\n}\n",
        ] {
            let err = format!("{:#}", parse(text).unwrap_err());
            assert!(err.contains("#true, #false, or one string"), "{err}");
        }
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
