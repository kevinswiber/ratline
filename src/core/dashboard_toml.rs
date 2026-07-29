//! The TOML constructor. Mirror structs, `deny_unknown_fields`, one
//! mapping to [`DashboardFile`] — no validation lives here (that is
//! `dashboard_file::into_registry`, the one validation path).

use anyhow::{Context, anyhow};

use crate::core::dashboard_file::{DashboardFile, PaneDecl};

pub fn parse(text: &str) -> anyhow::Result<DashboardFile> {
    let file: File = toml::from_str(text).context("reading the dashboard TOML")?;
    let defaults = file.defaults.unwrap_or_default();
    let default_shell = defaults.shell.unwrap_or(false);
    Ok(DashboardFile {
        gap: file.gap,
        row_gap: file.row_gap,
        defaults: pane_decl(&defaults, default_shell)?,
        panes: file
            .panes
            .iter()
            .map(|pane| pane_decl(pane, pane.shell.unwrap_or(default_shell)))
            .collect::<anyhow::Result<Vec<_>>>()?,
        layout: file.layout.map(|layout| {
            layout
                .rows
                .into_iter()
                .map(|row| match row {
                    Row::One(name) => vec![name],
                    Row::Many(names) => names,
                })
                .collect()
        }),
    })
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct File {
    gap: Option<usize>,
    row_gap: Option<usize>,
    defaults: Option<Pane>,
    /// `[[pane]]` reads singular in a file and plural in the model.
    #[serde(default, rename = "pane")]
    panes: Vec<Pane>,
    layout: Option<Layout>,
}

#[derive(Default, serde::Deserialize)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct Pane {
    name: Option<String>,
    command: Option<Command>,
    shell: Option<bool>,
    interval: Option<String>,
    trigger: Option<Vec<String>>,
    trigger_debounce: Option<String>,
    height: Option<u16>,
    width: Option<String>,
    overflow: Option<String>,
    border: Option<String>,
    padding: Option<String>,
    title: Option<String>,
    chrome: Option<bool>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Layout {
    rows: Vec<Row>,
}

#[derive(serde::Deserialize)]
#[serde(untagged)]
enum Row {
    One(String),
    Many(Vec<String>),
}

#[derive(Clone, serde::Deserialize)]
#[serde(untagged)]
enum Command {
    /// One string: a script under `shell`, otherwise a shell-words
    /// split.
    One(String),
    /// An array: argv, verbatim, no splitting.
    Many(Vec<String>),
}

/// `shell` is the ONE thing a parser resolves against `[defaults]`,
/// because whether a command string is split depends on it and
/// `PaneDecl.command` is already a vec by the time defaults resolve.
/// Fallible: an unbalanced command string is a parse error naming the
/// pane, never a silently-misread declaration.
fn pane_decl(pane: &Pane, shell: bool) -> anyhow::Result<PaneDecl> {
    let label = pane.name.as_deref().unwrap_or("defaults");
    Ok(PaneDecl {
        name: pane.name.clone(),
        command: pane
            .command
            .clone()
            .map(|command| words(command, shell, label))
            .transpose()?,
        shell: pane.shell,
        interval: pane.interval.clone(),
        trigger: pane.trigger.clone(),
        trigger_debounce: pane.trigger_debounce.clone(),
        height: pane.height,
        width: pane.width.clone(),
        overflow: pane.overflow.clone(),
        border: pane.border.clone(),
        padding: pane.padding.clone(),
        title: pane.title.clone(),
        chrome: pane.chrome,
    })
}

fn words(command: Command, shell: bool, pane: &str) -> anyhow::Result<Vec<String>> {
    match command {
        // The script survives byte-for-byte as one word.
        Command::One(script) if shell => Ok(vec![script]),
        // An unbalanced string PROPAGATES as a teaching error naming
        // the pane. It must not fall back to "the whole line as one
        // word": downstream validation checks only non-emptiness, so a
        // swallowed split error would let a malformed declaration
        // survive to a spawn.
        Command::One(line) => shell_words::split(&line)
            .map_err(|err| anyhow!("pane {pane:?}: command has unbalanced quoting ({err})")),
        Command::Many(argv) => Ok(argv),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unknown_toml_key_is_rejected() {
        // deny_unknown_fields gives the typo-teaching error for free:
        // serde names the key it did not expect.
        let err = format!(
            "{:#}",
            parse("[[pane]]\nname = \"a\"\ncomand = \"date\"\nheight = 3\n").unwrap_err()
        );
        assert!(err.contains("comand"), "{err}");
    }

    #[test]
    fn a_command_array_skips_word_splitting() {
        let file = parse(
            "[[pane]]\nname = \"a\"\nheight = 3\ncommand = [\"sh\", \"-c\", \"echo one two\"]\n",
        )
        .expect("parses");
        assert_eq!(
            file.panes[0].command,
            Some(vec![
                "sh".to_string(),
                "-c".to_string(),
                "echo one two".to_string()
            ])
        );
    }

    #[test]
    fn a_command_string_is_word_split() {
        let file =
            parse("[[pane]]\nname = \"a\"\nheight = 3\ncommand = \"git log --oneline -5\"\n")
                .expect("parses");
        assert_eq!(
            file.panes[0].command,
            Some(vec![
                "git".to_string(),
                "log".to_string(),
                "--oneline".to_string(),
                "-5".to_string()
            ])
        );
        // shell-words, not whitespace: a quoted argument stays one word.
        let quoted =
            parse("[[pane]]\nname = \"a\"\nheight = 3\ncommand = \"rat style 'two words'\"\n")
                .expect("parses");
        assert_eq!(
            quoted.panes[0].command,
            Some(vec![
                "rat".to_string(),
                "style".to_string(),
                "two words".to_string()
            ])
        );
    }

    #[test]
    fn a_shell_pane_keeps_its_script_as_one_word() {
        // Split-then-join destroys quoting, so a shell script is never
        // split — including when `shell` came from [defaults].
        // Inside a TOML basic string a backslash escapes, so the
        // script's literal \n arrives as \\n in the file.
        let file = parse(
            "[defaults]\nshell = true\n\n[[pane]]\nname = \"a\"\nheight = 3\ncommand = \"date +%H:%M | tr -d '\\\\n'\"\n",
        )
        .expect("parses");
        assert_eq!(
            file.panes[0].command,
            Some(vec!["date +%H:%M | tr -d '\\n'".to_string()])
        );
    }

    #[test]
    fn an_unbalanced_command_string_is_rejected_naming_the_pane() {
        // A swallowed shell_words error would survive into_registry
        // (which checks only non-emptiness) and reach a spawn — the
        // fail-before-spawn boundary. Both grammars, same behavior.
        let toml = parse("[[pane]]\nname = \"a\"\nheight = 3\ncommand = \"echo 'oops\"\n")
            .expect_err("unbalanced quoting is a parse error");
        assert!(toml.to_string().contains("\"a\""), "names the pane: {toml}");
        let kdl = crate::core::dashboard_kdl::parse(
            "pane \"a\" {\n    height 3\n    command \"echo 'oops\"\n}\n",
        )
        .expect_err("unbalanced quoting is a parse error");
        assert!(kdl.to_string().contains("\"a\""), "names the pane: {kdl}");
    }

    #[test]
    fn a_negative_gap_is_rejected_by_both_grammars() {
        // `as usize` would wrap -1 into a repeat count near usize::MAX,
        // which `" ".repeat(gap)` would try to allocate. Checked
        // conversions only; the KDL error names the field.
        assert!(
            parse("gap = -1\n").is_err(),
            "serde usize rejects negatives"
        );
        let kdl = crate::core::dashboard_kdl::parse("gap -1\n")
            .expect_err("negative gap is a parse error");
        assert!(kdl.to_string().contains("gap"), "names the field: {kdl}");
        assert!(crate::core::dashboard_kdl::parse("row-gap -1\n").is_err());
    }

    #[test]
    fn a_negative_height_is_rejected() {
        // Same class, u16 flavor.
        assert!(parse("[[pane]]\nname = \"a\"\nheight = -3\ncommand = \"date\"\n").is_err());
        assert!(
            crate::core::dashboard_kdl::parse(
                "pane \"a\" {\n    height -3\n    command \"date\"\n}\n"
            )
            .is_err()
        );
    }
}
