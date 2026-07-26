use crate::color::EnvSource;

/// A resolved pager invocation. Policy mirrors bat/delta: RAT_PAGER, then
/// PAGER, then `less`; when the pager is less, `-R` is ensured so SGR color
/// passes through (`-K` joins it only when the user supplied no arguments).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagerCommand {
    pub bin: String,
    pub args: Vec<String>,
}

pub fn resolve_pager(env: &dyn EnvSource) -> Option<PagerCommand> {
    let configured = ["RAT_PAGER", "PAGER"]
        .iter()
        .filter_map(|key| env.get(key))
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty());

    let command = match configured {
        Some(value) => match shell_words::split(&value) {
            Ok(words) if !words.is_empty() => {
                let mut iter = words.into_iter();
                let bin = iter.next().expect("non-empty");
                // Using rat as its own pager would recurse; fall back.
                if base_name(&bin) == "rat" {
                    default_less()
                } else {
                    PagerCommand {
                        bin,
                        args: iter.collect(),
                    }
                }
            }
            _ => default_less(),
        },
        None => default_less(),
    };

    let mut command = command;
    if base_name(&command.bin) == "less" && !command.args.iter().any(|a| a == "-R") {
        command.args.push("-R".to_string());
    }
    Some(command)
}

fn default_less() -> PagerCommand {
    PagerCommand {
        bin: "less".to_string(),
        args: vec!["-R".to_string(), "-K".to_string()],
    }
}

/// Lowercased file name with any .exe suffix dropped, so "C:/tools/LESS.EXE"
/// counts as less.
fn base_name(bin: &str) -> String {
    let name = bin.rsplit(['/', '\\']).next().unwrap_or(bin).to_lowercase();
    name.strip_suffix(".exe").unwrap_or(&name).to_string()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::color::MapEnv;

    fn env(pairs: &[(&str, &str)]) -> MapEnv {
        MapEnv(
            pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<HashMap<_, _>>(),
        )
    }

    #[test]
    fn defaults_to_less_with_color_flags() {
        let pager = resolve_pager(&env(&[])).unwrap();
        assert_eq!(pager.bin, "less");
        assert_eq!(pager.args, vec!["-R", "-K"]);
    }

    #[test]
    fn rat_pager_beats_pager() {
        let e = env(&[("RAT_PAGER", "moar"), ("PAGER", "most")]);
        assert_eq!(resolve_pager(&e).unwrap().bin, "moar");
    }

    #[test]
    fn empty_values_fall_through() {
        let e = env(&[("RAT_PAGER", ""), ("PAGER", "   ")]);
        assert_eq!(resolve_pager(&e).unwrap().bin, "less");
    }

    #[test]
    fn quoted_args_parse() {
        let e = env(&[("PAGER", "sh -c 'less -R'")]);
        let pager = resolve_pager(&e).unwrap();
        assert_eq!(pager.bin, "sh");
        assert_eq!(pager.args, vec!["-c", "less -R"]);
    }

    #[test]
    fn user_less_args_kept_and_r_ensured_once() {
        let e = env(&[("PAGER", "less -X")]);
        let pager = resolve_pager(&e).unwrap();
        assert_eq!(pager.args, vec!["-X", "-R"]);
        let e = env(&[("PAGER", "less -R -F")]);
        let pager = resolve_pager(&e).unwrap();
        assert_eq!(pager.args, vec!["-R", "-F"]);
    }

    #[test]
    fn windows_style_less_path_is_recognized() {
        let e = env(&[("PAGER", "C:/tools/less.exe -F")]);
        let pager = resolve_pager(&e).unwrap();
        assert!(pager.args.contains(&"-R".to_string()));
    }

    #[test]
    fn non_less_pagers_are_untouched() {
        let e = env(&[("PAGER", "moar -no-linenumbers")]);
        let pager = resolve_pager(&e).unwrap();
        assert_eq!(pager.args, vec!["-no-linenumbers"]);
    }

    #[test]
    fn rat_as_pager_falls_back_to_less() {
        let e = env(&[("PAGER", "rat")]);
        assert_eq!(resolve_pager(&e).unwrap().bin, "less");
    }

    #[test]
    fn unparseable_value_falls_back_to_less() {
        let e = env(&[("PAGER", "less 'unclosed")]);
        assert_eq!(resolve_pager(&e).unwrap().bin, "less");
    }
}
