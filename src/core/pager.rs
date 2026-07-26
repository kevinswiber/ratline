use crate::color::EnvSource;

/// A resolved pager invocation. Policy mirrors bat/delta: RAT_PAGER, then
/// PAGER, then `less`; when the pager is less, `-R` is ensured so SGR color
/// passes through (`-K` joins it only when the user supplied no arguments).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagerCommand {
    pub bin: String,
    pub args: Vec<String>,
}

/// Pager candidates in launch order; the first that spawns wins. A
/// configured pager is trusted as-is (a missing one should be noticed, not
/// papered over), while the default chain appends the stock Windows pager
/// so `v` works out of the box without less installed.
pub fn resolve_pagers(env: &dyn EnvSource) -> Vec<PagerCommand> {
    resolve_with(env, cfg!(windows))
}

fn resolve_with(env: &dyn EnvSource, on_windows: bool) -> Vec<PagerCommand> {
    let configured = ["RAT_PAGER", "PAGER"]
        .iter()
        .filter_map(|key| env.get(key))
        .map(|v| v.trim().to_string())
        .find(|v| !v.is_empty());

    let mut candidates = match configured {
        Some(value) => match shell_words::split(&value) {
            Ok(words) if !words.is_empty() => {
                let mut iter = words.into_iter();
                let bin = iter.next().expect("non-empty");
                // Using rat as its own pager would recurse; fall back.
                if base_name(&bin) == "rat" {
                    default_candidates(env, on_windows)
                } else {
                    vec![PagerCommand {
                        bin,
                        args: iter.collect(),
                    }]
                }
            }
            _ => default_candidates(env, on_windows),
        },
        None => default_candidates(env, on_windows),
    };

    for command in &mut candidates {
        if base_name(&command.bin) == "less" && !command.args.iter().any(|a| a == "-R") {
            command.args.push("-R".to_string());
        }
    }
    candidates
}

fn default_candidates(env: &dyn EnvSource, on_windows: bool) -> Vec<PagerCommand> {
    let mut candidates = vec![PagerCommand {
        bin: "less".to_string(),
        args: vec!["-R".to_string(), "-K".to_string()],
    }];
    if on_windows {
        // more.com is always present; a full path dodges CreateProcess's
        // exe-only PATH resolution for the extensionless name.
        let root = env
            .get("SystemRoot")
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| r"C:\WINDOWS".to_string());
        candidates.push(PagerCommand {
            bin: format!(r"{root}\System32\more.com"),
            args: Vec::new(),
        });
    }
    candidates
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
        let pagers = resolve_pagers(&env(&[]));
        assert_eq!(pagers[0].bin, "less");
        assert_eq!(pagers[0].args, vec!["-R", "-K"]);
    }

    #[test]
    fn rat_pager_beats_pager() {
        let e = env(&[("RAT_PAGER", "moar"), ("PAGER", "most")]);
        assert_eq!(resolve_pagers(&e)[0].bin, "moar");
    }

    #[test]
    fn empty_values_fall_through() {
        let e = env(&[("RAT_PAGER", ""), ("PAGER", "   ")]);
        assert_eq!(resolve_pagers(&e)[0].bin, "less");
    }

    #[test]
    fn quoted_args_parse() {
        let e = env(&[("PAGER", "sh -c 'less -R'")]);
        let pagers = resolve_pagers(&e);
        assert_eq!(pagers[0].bin, "sh");
        assert_eq!(pagers[0].args, vec!["-c", "less -R"]);
    }

    #[test]
    fn user_less_args_kept_and_r_ensured_once() {
        let e = env(&[("PAGER", "less -X")]);
        assert_eq!(resolve_pagers(&e)[0].args, vec!["-X", "-R"]);
        let e = env(&[("PAGER", "less -R -F")]);
        assert_eq!(resolve_pagers(&e)[0].args, vec!["-R", "-F"]);
    }

    #[test]
    fn windows_style_less_path_is_recognized() {
        let e = env(&[("PAGER", "C:/tools/less.exe -F")]);
        assert!(resolve_pagers(&e)[0].args.contains(&"-R".to_string()));
    }

    #[test]
    fn non_less_pagers_are_untouched() {
        let e = env(&[("PAGER", "moar -no-linenumbers")]);
        assert_eq!(resolve_pagers(&e)[0].args, vec!["-no-linenumbers"]);
    }

    #[test]
    fn rat_as_pager_falls_back_to_less() {
        let e = env(&[("PAGER", "rat")]);
        assert_eq!(resolve_pagers(&e)[0].bin, "less");
    }

    #[test]
    fn unparseable_value_falls_back_to_less() {
        let e = env(&[("PAGER", "less 'unclosed")]);
        assert_eq!(resolve_pagers(&e)[0].bin, "less");
    }

    #[test]
    fn windows_default_falls_back_to_the_stock_pager() {
        let pagers = resolve_with(&env(&[]), true);
        assert_eq!(pagers.len(), 2);
        assert_eq!(pagers[0].bin, "less");
        assert_eq!(pagers[1].bin, r"C:\WINDOWS\System32\more.com");
        assert!(pagers[1].args.is_empty());
    }

    #[test]
    fn windows_fallback_honors_systemroot() {
        let e = env(&[("SystemRoot", r"D:\Win")]);
        let pagers = resolve_with(&e, true);
        assert_eq!(pagers[1].bin, r"D:\Win\System32\more.com");
    }

    #[test]
    fn unix_default_has_no_fallback() {
        assert_eq!(resolve_with(&env(&[]), false).len(), 1);
    }

    #[test]
    fn configured_pagers_get_no_fallback() {
        let e = env(&[("RAT_PAGER", "moar")]);
        assert_eq!(resolve_with(&e, true).len(), 1);
        let e = env(&[("PAGER", "less")]);
        assert_eq!(resolve_with(&e, true).len(), 1);
    }

    #[test]
    fn rat_as_pager_regains_the_windows_fallback() {
        let e = env(&[("PAGER", "rat")]);
        assert_eq!(resolve_with(&e, true).len(), 2);
    }
}
