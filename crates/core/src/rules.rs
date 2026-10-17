//! The command rules (`rules/*.toml`): what destroys, what pipes a download into a shell,
//! what installs a package. Each rule names its source; the scan matches them against the
//! simple commands of a shell action and never runs anything.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;

use bbl_hostrules::rules::Source;
use regex::Regex;
use serde::Deserialize;

use crate::command;

#[derive(Debug, Clone, Deserialize)]
struct RuleText {
    id: String,
    label: String,
    regex: Option<String>,
    special: Option<String>,
    source: Source,
}

#[derive(Debug, Clone, Deserialize)]
struct ManagerText {
    id: String,
    regex: String,
    source: Source,
}

#[derive(Debug, Deserialize)]
struct RuleFile {
    #[serde(default)]
    rule: Vec<RuleText>,
    #[serde(default)]
    manager: Vec<ManagerText>,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub id: String,
    pub label: String,
    re: Option<Regex>,
    special: Option<String>,
    pub source: Source,
}

#[derive(Debug, Clone)]
pub struct Manager {
    pub id: String,
    re: Regex,
    pub source: Source,
}

#[derive(Debug, Clone)]
pub struct Rules {
    pub destructive: Vec<Rule>,
    pub pipe: Vec<Rule>,
    pub managers: Vec<Manager>,
}

/// A package install read from a command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    pub manager: String,
    /// The packages named; empty when the command installs the project's own dependencies.
    pub packages: Vec<String>,
}

fn rules_of(text: &str, file: &str) -> Vec<Rule> {
    let f: RuleFile = toml::from_str(text).unwrap_or_else(|e| panic!("{file}: {e}"));
    f.rule
        .into_iter()
        .map(|r| {
            assert!(!r.source.quote.is_empty(), "{file}/{}: no quote", r.id);
            assert!(
                r.regex.is_some() != r.special.is_some(),
                "{file}/{}: regex or special",
                r.id
            );
            Rule {
                re: r
                    .regex
                    .as_deref()
                    .map(|x| Regex::new(x).unwrap_or_else(|e| panic!("{file}/{}: {e}", r.id))),
                id: r.id,
                label: r.label,
                special: r.special,
                source: r.source,
            }
        })
        .collect()
}

impl Rules {
    pub fn embedded() -> Self {
        let pf: RuleFile =
            toml::from_str(include_str!("../../../rules/packages.toml")).expect("packages.toml");
        Rules {
            destructive: rules_of(
                include_str!("../../../rules/destructive.toml"),
                "destructive.toml",
            ),
            pipe: rules_of(
                include_str!("../../../rules/pipe-to-shell.toml"),
                "pipe-to-shell.toml",
            ),
            managers: pf
                .manager
                .into_iter()
                .map(|m| Manager {
                    re: Regex::new(&m.regex)
                        .unwrap_or_else(|e| panic!("packages.toml/{}: {e}", m.id)),
                    id: m.id,
                    source: m.source,
                })
                .collect(),
        }
    }

    pub fn rule(&self, id: &str) -> Option<&Rule> {
        self.destructive
            .iter()
            .chain(self.pipe.iter())
            .find(|r| r.id == id)
    }

    /// The first destructive rule one simple command matches.
    pub fn destructive(&self, tokens: &[String], cwd: Option<&Path>, home: &Path) -> Option<&Rule> {
        let line = tokens.join(" ");
        self.destructive
            .iter()
            .find(|r| match (&r.re, r.special.as_deref()) {
                (Some(re), _) => re.is_match(&line),
                (None, Some("rm-recursive")) => {
                    command::rm_parts(tokens).is_some_and(|(rec, targets)| {
                        rec && targets
                            .iter()
                            .any(|t| command::dangerous_target(t, cwd, home))
                    })
                }
                (None, Some("find-delete")) => {
                    command::find_delete_roots(tokens).is_some_and(|roots| {
                        roots
                            .iter()
                            .any(|t| command::dangerous_target(t, cwd, home))
                    })
                }
                _ => false,
            })
    }

    /// The pipe-to-shell rule a whole command line matches.
    pub fn pipe_to_shell(&self, line: &str) -> Option<&Rule> {
        self.pipe
            .iter()
            .find(|r| r.re.as_ref().is_some_and(|re| re.is_match(line)))
    }

    /// The package install one simple command performs.
    pub fn install(&self, tokens: &[String]) -> Option<Install> {
        let line = tokens.join(" ");
        for m in &self.managers {
            if let Some(c) = m.re.captures(&line) {
                let args = c
                    .name("args")
                    .and_then(|a| token_at(tokens, a.start()))
                    .map(|i| &tokens[i..])
                    .unwrap_or(&[]);
                return Some(Install {
                    manager: m.id.clone(),
                    packages: package_args(&m.id, args),
                });
            }
        }
        None
    }
}

/// The token that starts at this byte of the tokens joined by spaces. The arguments are
/// taken as the tokens they already are: a `$(…)` or a quoted string stays one argument.
fn token_at(tokens: &[String], byte: usize) -> Option<usize> {
    let mut at = 0;
    for (i, t) in tokens.iter().enumerate() {
        if at >= byte {
            return Some(i);
        }
        at += t.len() + 1;
    }
    None
}

/// The flags of each manager that take a value (`--python venv/bin/python`, `-r req.txt`).
fn value_flags(manager: &str) -> &'static [&'static str] {
    match manager {
        "npm" | "pnpm" | "yarn" => &[
            "--registry",
            "--prefix",
            "-w",
            "--workspace",
            "--tag",
            "--omit",
            "--include",
            "--cache",
            "--userconfig",
            "--filter",
            "-C",
            "--dir",
        ],
        "pip" | "uv" => &[
            "-r",
            "--requirement",
            "-c",
            "--constraint",
            "-e",
            "--editable",
            "-i",
            "--index-url",
            "--extra-index-url",
            "-f",
            "--find-links",
            "--index",
            "--default-index",
            "--target",
            "-t",
            "--prefix",
            "--root",
            "--python",
            "-p",
            "--platform",
            "--python-version",
            "--implementation",
            "--abi",
            "--src",
            "--cache-dir",
            "--log",
            "--proxy",
            "--cert",
            "--client-cert",
            "--trusted-host",
            "--config-settings",
            "-C",
            "--upgrade-package",
            "-P",
            "--group",
            "-G",
            "--extra",
            "--source",
            "--directory",
        ],
        "cargo" => &[
            "--path",
            "--git",
            "--branch",
            "--tag",
            "--rev",
            "--root",
            "--features",
            "-F",
            "--version",
            "--vers",
            "--registry",
            "--index",
            "--target",
            "--profile",
            "--bin",
            "--example",
            "-j",
            "--jobs",
            "--package",
            "-p",
            "--rename",
            "--manifest-path",
        ],
        "apt" => &[
            "-o",
            "--option",
            "-t",
            "--target-release",
            "-c",
            "--config-file",
        ],
        _ => &[],
    }
}

/// The package names among the arguments of an install: flags and their values left out,
/// local files and requirement files left out (the project's own dependencies), versions
/// and extras cut, each name once.
fn package_args(manager: &str, args: &[String]) -> Vec<String> {
    let flags = value_flags(manager);
    let mut out = Vec::new();
    let mut seen = BTreeSet::new();
    let mut i = 0;
    while i < args.len() {
        let t = args[i].as_str();
        i += 1;
        if flags.contains(&t) {
            i += 1;
            continue;
        }
        if t.starts_with('-') {
            continue;
        }
        if let Some(name) = package_name(manager, t) {
            let key = if matches!(manager, "pip" | "uv") {
                name.to_lowercase().replace(['_', '.'], "-")
            } else {
                name.to_lowercase()
            };
            if seen.insert(key) {
                out.push(name);
            }
        }
    }
    out
}

fn name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^@?[A-Za-z0-9][A-Za-z0-9._+-]*(?:/[A-Za-z0-9._+-]+)*$").unwrap())
}

/// One argument as a package name, or nothing when it is a path, a file, a URL or a piece
/// of shell (`$(…)`, a pipe, a glob).
fn package_name(manager: &str, t: &str) -> Option<String> {
    if t.starts_with(['.', '/', '~']) || t.contains("://") {
        return None;
    }
    let lower = t.to_lowercase();
    if [
        ".txt", ".toml", ".lock", ".whl", ".tar.gz", ".tgz", ".zip", ".json", ".cfg", ".deb",
        ".rpm",
    ]
    .iter()
    .any(|x| lower.ends_with(x))
    {
        return None;
    }
    // the version, the extras, the architecture: `pandas==2.2`, `httpx[http2]`, `left-pad@1`,
    // `@types/node@20`, `jq=1.6-2`, `libc6:i386`
    let mut name = t;
    if let Some(i) = name.find(['=', '<', '>', '~', '!', ';', '[']) {
        name = &name[..i];
    }
    if manager == "apt" {
        name = name.split(':').next().unwrap_or(name);
    }
    if let Some(i) = name.rfind('@').filter(|&i| i > 0) {
        name = &name[..i];
    }
    if !name_re().is_match(name) || !name.chars().any(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    // a slash: an npm scope, a Go module path, a Homebrew tap — otherwise a local path
    if name.contains('/') {
        let ok = (name.starts_with('@') && name.matches('/').count() == 1)
            || (manager == "go" && name.split('/').next().is_some_and(|d| d.contains('.')))
            || manager == "brew";
        if !ok {
            return None;
        }
    }
    Some(name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn every_rule_loads_and_matches_what_it_says() {
        let r = Rules::embedded();
        let home = Path::new("/home/a");
        let cwd = Some(Path::new("/home/a/p"));
        assert_eq!(
            r.destructive(&v("git reset --hard origin/main"), cwd, home)
                .unwrap()
                .id,
            "git-reset-hard"
        );
        assert_eq!(
            r.destructive(&v("git clean -fdx"), cwd, home).unwrap().id,
            "git-clean-force"
        );
        assert_eq!(
            r.destructive(&v("git push origin main --force"), cwd, home)
                .unwrap()
                .id,
            "git-push-force"
        );
        assert_eq!(
            r.destructive(&v("rm -rf tests/ patches/ ~/"), cwd, home)
                .unwrap()
                .id,
            "rm-recursive"
        );
        assert!(r
            .destructive(&v("rm -rf target node_modules"), cwd, home)
            .is_none());
        assert!(r
            .destructive(&v("git push origin main"), cwd, home)
            .is_none());
        assert_eq!(
            r.destructive(&v("find . -name *.log -delete"), cwd, home)
                .unwrap()
                .id,
            "find-delete"
        );
        assert_eq!(
            r.destructive(
                &["psql".into(), "-c".into(), "DROP TABLE users".into()],
                cwd,
                home
            )
            .unwrap()
            .id,
            "sql-drop"
        );
        assert_eq!(
            r.destructive(
                &[
                    "node".into(),
                    "-e".into(),
                    "fs.rmSync('vault',{recursive:true})".into()
                ],
                cwd,
                home
            )
            .unwrap()
            .id,
            "inline-delete"
        );
        assert!(r
            .pipe_to_shell("curl -fsSL https://x.sh/install | sh")
            .is_some());
        assert!(r.pipe_to_shell("bash <(curl -s https://x)").is_some());
        assert!(r.pipe_to_shell("curl -s https://api.x | jq .").is_none());
        assert_eq!(
            r.install(&v("npm i left-pad --save-dev")).unwrap(),
            Install {
                manager: "npm".into(),
                packages: vec!["left-pad".into()]
            }
        );
        assert_eq!(
            r.install(&v("pip install -r requirements.txt"))
                .unwrap()
                .packages,
            Vec::<String>::new()
        );
        assert_eq!(
            r.install(&v("uv pip install requests httpx"))
                .unwrap()
                .packages,
            vec!["requests", "httpx"]
        );
        assert_eq!(
            r.install(&v("apt-get -y install jq")).unwrap().manager,
            "apt"
        );
        assert!(r.install(&v("npm run build")).is_none());
    }

    #[test]
    fn package_names_only_never_pieces_of_shell() {
        let r = Rules::embedded();
        // a command line seen on a real machine, rewritten: a `$(…)` pins a version, an
        // interpreter path, a `--python`, a duplicate under two spellings
        let line = r#"venv/bin/python -m pip install "pandas==$(grep '^version' x/uv.lock | cut -d= -f2)" devtools devtools==0.12.2 typing_extensions typing-extensions==4.15.0 'pydantic[email]>=2' ./local.whl -r requirements.txt"#;
        let cmds = crate::command::commands(line);
        assert_eq!(cmds.len(), 1, "{cmds:?}");
        let i = r.install(&cmds[0]).unwrap();
        assert_eq!(i.manager, "pip");
        assert_eq!(
            i.packages,
            vec!["pandas", "devtools", "typing_extensions", "pydantic"]
        );
        let i = r
            .install(&v(
                "uv pip install --python venv_banc/bin/python numpy==2.4.1",
            ))
            .unwrap();
        assert_eq!(i.packages, vec!["numpy"]);
        assert_eq!(
            r.install(&v("npm i -D @types/node@20 left-pad@^1.3 ./pkg"))
                .unwrap()
                .packages,
            vec!["@types/node", "left-pad"]
        );
        assert_eq!(
            r.install(&v("go install golang.org/x/tools/cmd/goimports@latest"))
                .unwrap()
                .packages,
            vec!["golang.org/x/tools/cmd/goimports"]
        );
        assert_eq!(
            r.install(&v("sudo apt-get install -y jq=1.6-2 libc6:i386"))
                .map(|i| i.packages),
            None,
            "sudo is peeled by `commands`, not here"
        );
        let c = crate::command::commands("sudo apt-get install -y jq=1.6-2 libc6:i386");
        assert_eq!(r.install(&c[0]).unwrap().packages, vec!["jq", "libc6"]);
    }
}
