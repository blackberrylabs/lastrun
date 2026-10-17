//! Shell commands as an agent wrote them: split into simple commands, unwrapped (`sudo`,
//! `env`, `VAR=x`, `bash -c "…"`), and masked. Nothing here runs anything; the parsing is a
//! reading of the text, tolerant of what it does not understand.

use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

/// Split a command line into simple commands at `&&`, `||`, `;`, `|`, `&` and newlines
/// outside quotes. Each simple command comes back as its tokens, quotes removed.
pub fn simple_commands(line: &str) -> Vec<Vec<String>> {
    let mut out: Vec<Vec<String>> = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    let mut tok = String::new();
    let mut in_tok = false;
    let mut chars = line.chars().peekable();
    let mut quote: Option<char> = None;
    // `$(…)` and backticks stay inside the token they open.
    let mut depth = 0usize;
    let flush_tok = |tok: &mut String, in_tok: &mut bool, cur: &mut Vec<String>| {
        if *in_tok {
            cur.push(std::mem::take(tok));
            *in_tok = false;
        }
    };
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else if c == '\\' && q == '"' {
                if let Some(n) = chars.next() {
                    tok.push(n);
                }
            } else {
                tok.push(c);
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                in_tok = true;
            }
            '\\' => {
                if let Some(n) = chars.next() {
                    if n != '\n' {
                        tok.push(n);
                        in_tok = true;
                    }
                }
            }
            '$' if chars.peek() == Some(&'(') => {
                tok.push(c);
                tok.push(chars.next().unwrap());
                depth += 1;
                in_tok = true;
            }
            ')' if depth > 0 => {
                tok.push(c);
                depth -= 1;
            }
            ' ' | '\t' if depth == 0 => flush_tok(&mut tok, &mut in_tok, &mut cur),
            ';' | '\n' | '|' | '&' if depth == 0 => {
                flush_tok(&mut tok, &mut in_tok, &mut cur);
                // `&&`, `||`, `|&`, `;;` are one separator
                if (c == '&' || c == '|') && matches!(chars.peek(), Some('&') | Some('|')) {
                    chars.next();
                }
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            '#' if !in_tok && depth == 0 => {
                // a comment runs to the end of the line
                for n in chars.by_ref() {
                    if n == '\n' {
                        break;
                    }
                }
                flush_tok(&mut tok, &mut in_tok, &mut cur);
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            _ => {
                tok.push(c);
                in_tok = true;
            }
        }
    }
    flush_tok(&mut tok, &mut in_tok, &mut cur);
    if !cur.is_empty() {
        out.push(cur);
    }
    // redirections are not arguments
    out.into_iter()
        .map(strip_redirections)
        .filter(|c| !c.is_empty())
        .collect()
}

fn strip_redirections(cmd: Vec<String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut skip_next = false;
    for t in cmd {
        if skip_next {
            skip_next = false;
            continue;
        }
        let bare = t.trim_start_matches(|c: char| c.is_ascii_digit());
        if matches!(bare, ">" | ">>" | "<" | "<<" | "<<<" | ">|" | "&>" | "&>>") {
            skip_next = true;
            continue;
        }
        if bare.starts_with(">&")
            || bare.starts_with("<&")
            || (bare.starts_with('>') && bare.len() > 1)
            || (bare.starts_with('<') && !bare.starts_with("<(") && bare.len() > 1)
        {
            continue;
        }
        out.push(t);
    }
    out
}

/// Peel what runs a command without being it: `sudo` and its flags, `env` and assignments,
/// `VAR=x`, `time`, `nohup`, `exec`, `command`, `nice`, `timeout N`. A `bash -c "…"` or
/// `sh -lc "…"` is opened and its own simple commands returned.
pub fn unwrap(tokens: &[String]) -> Vec<Vec<String>> {
    let mut i = 0;
    while i < tokens.len() {
        let t = tokens[i].as_str();
        let is_assign = t.contains('=')
            && !t.starts_with('-')
            && t.split('=').next().is_some_and(|k| {
                !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            });
        match t {
            "sudo" | "doas" => {
                i += 1;
                while i < tokens.len() && tokens[i].starts_with('-') {
                    // `-u user`, `-g group` take a value
                    let takes = matches!(
                        tokens[i].as_str(),
                        "-u" | "-g" | "-U" | "-C" | "-D" | "-h" | "-p" | "-r" | "-t"
                    );
                    i += if takes { 2 } else { 1 };
                }
            }
            "env" => {
                i += 1;
                while i < tokens.len() && (tokens[i].starts_with('-') || tokens[i].contains('=')) {
                    i += 1;
                }
            }
            "time" | "nohup" | "exec" | "command" | "builtin" | "stdbuf" => i += 1,
            "nice" | "ionice" => {
                i += 1;
                while i < tokens.len() && tokens[i].starts_with('-') {
                    i += 1;
                    if i < tokens.len() && tokens[i].parse::<i64>().is_ok() {
                        i += 1;
                    }
                }
            }
            "timeout" => {
                i += 1;
                while i < tokens.len() && tokens[i].starts_with('-') {
                    i += 1;
                }
                if i < tokens.len() {
                    i += 1; // the duration
                }
            }
            _ if is_assign => i += 1,
            _ => break,
        }
    }
    let rest: Vec<String> = tokens[i.min(tokens.len())..].to_vec();
    if rest.is_empty() {
        return Vec::new();
    }
    let prog = program_name(&rest[0]);
    if matches!(prog, "bash" | "sh" | "zsh" | "dash" | "ksh" | "fish") {
        // `bash -c 'cmd'`, `bash -lc "cmd"`, `bash -e -c cmd`
        let mut j = 1;
        while j < rest.len() && rest[j].starts_with('-') {
            if rest[j].contains('c') && !rest[j].starts_with("--") {
                if let Some(script) = rest.get(j + 1) {
                    return simple_commands(script)
                        .into_iter()
                        .flat_map(|c| unwrap(&c))
                        .collect();
                }
            }
            j += 1;
        }
    }
    vec![rest]
}

/// Every simple command of a command line, unwrapped.
pub fn commands(line: &str) -> Vec<Vec<String>> {
    simple_commands(line)
        .iter()
        .flat_map(|c| unwrap(c))
        .collect()
}

/// `/usr/bin/rm` → `rm`.
pub fn program_name(t: &str) -> &str {
    t.rsplit('/').next().unwrap_or(t)
}

/// A command as the host wrote it when it is an argv array (Codex: `["bash", "-lc", "…"]`):
/// the script when it is a shell wrapper, the words joined otherwise.
pub fn from_argv(argv: &[String]) -> String {
    if argv.len() >= 3
        && matches!(program_name(&argv[0]), "bash" | "sh" | "zsh")
        && argv[1].starts_with('-')
        && argv[1].contains('c')
    {
        return argv[2..].join(" ");
    }
    argv.iter()
        .map(|a| {
            if a.contains(' ') {
                format!("'{a}'")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------- secrets

/// What is shown in place of a value under a secret-looking name: none of its characters.
pub const DOTS: &str = "••••••••";

/// A name that says its value is a credential: it ends with KEY, TOKEN, SECRET, PASSWORD…
/// (`OPENAI_API_KEY`, `apiKey`, `access_token`, `PGPASSWORD`), not merely contains one
/// (`KEY_BY_PROVIDER`, `tokenizer`, `max_tokens`, `author`).
const NAME: &str = r"[A-Za-z0-9_.-]*(?:key|token|secret|password|passwd|pwd|credentials?)";

fn name_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(&format!(r"(?i)^{NAME}$")).unwrap())
}

fn assign_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(&format!(
            r#"(?i)\b({NAME})\b(['"]?\s*[=:]\s*)("[^"\n]*"|'[^'\n]*'|[^\s"'&;|\\]+)"#
        ))
        .unwrap()
    })
}

fn bearer_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b(bearer|basic)\s+([A-Za-z0-9._~+/=-]{16,})").unwrap())
}

fn identifier_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[A-Za-z_][A-Za-z0-9_]*(?:\.[A-Za-z_][A-Za-z0-9_]*)*$").unwrap())
}

/// Credential shapes with a known prefix. Pure hexadecimal is left out on purpose: commit
/// hashes are everywhere in a transcript and are not secrets.
fn shape_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(concat!(
            r"\b(",
            r"sk-(?:ant-|proj-|or-v1-)?[A-Za-z0-9_-]{20,}",
            r"|gh[pousr]_[A-Za-z0-9]{30,}",
            r"|github_pat_[A-Za-z0-9_]{30,}",
            r"|xox[abprs]-[A-Za-z0-9-]{10,}",
            r"|AKIA[0-9A-Z]{16}",
            r"|AIza[0-9A-Za-z_-]{30,}",
            r"|glpat-[A-Za-z0-9_-]{20,}",
            r"|gsk_[A-Za-z0-9]{30,}",
            r"|hf_[A-Za-z0-9]{30,}",
            r"|ntn_[A-Za-z0-9]{30,}",
            r"|oc_sk_[A-Za-z0-9]{20,}",
            r"|eyJ[A-Za-z0-9_-]{10,}\.eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}",
            r")"
        ))
        .unwrap()
    })
}

/// A value written as an example, not a credential: `sk-xxxx…`, `your-api-key`, a run of the
/// same character, the fake keys of test fixtures.
pub fn is_placeholder(v: &str) -> bool {
    let l = v.to_lowercase();
    let body = l.trim_start_matches(|c: char| !c.is_ascii_alphanumeric());
    [
        "xxxx",
        "your",
        "example",
        "placeholder",
        "dummy",
        "changeme",
        "redacted",
        "secret",
        "fake",
        "test",
        "sample",
        "0123456789",
        "abcdef0123",
        "…",
        "...",
        "<",
        "${",
        "$(",
    ]
    .iter()
    .any(|p| l.contains(p))
        || body
            .chars()
            .collect::<std::collections::BTreeSet<_>>()
            .len()
            <= 4
}

/// A value and the punctuation that closes the sentence or the structure around it.
fn split_tail(v: &str) -> (&str, &str) {
    let core = v.trim_end_matches([',', ';', ':', ')', ']', '}', '.']);
    (core, &v[core.len()..])
}

/// Is a value under a secret-looking name a credential rather than a word, a path, a
/// number, an identifier, a piece of code or a reference?
fn assigned_value_is_secret(v: &str) -> bool {
    let v = split_tail(v.trim_matches(|c| c == '"' || c == '\'')).0;
    if v.chars().count() < 12
        || v.starts_with(['$', '/', '.', '~', '<', '{', '[', '(', '-', '%'])
        || v.starts_with("http")
        || v.contains(|c: char| c.is_whitespace() || "\\()[]{}<>,`'\"".contains(c))
        || v.chars().all(|c| c.is_ascii_digit())
    {
        return false;
    }
    // `KEY_BY_PROVIDER_1`, `settings.api_key2`: a name in code, not a value
    if identifier_re().is_match(v) && (v.contains('_') || v.contains('.')) {
        return false;
    }
    // `src/auth/token_store2.rs`: a relative path; a base64 credential is long and dotless
    if v.contains('/') && (v.contains('.') || v.len() < 32) {
        return false;
    }
    let digits = v.chars().any(|c| c.is_ascii_digit());
    let letters = v.chars().any(|c| c.is_ascii_alphabetic());
    digits && letters && !is_placeholder(v)
}

fn fingerprint(v: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    v.hash(&mut h);
    h.finish()
}

/// One secret found in a text: the masked form only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    /// What may be shown: `sk-ant…Wx3a` for a known shape, `DB_PASSWORD=••••••••` for a
    /// value under a secret-looking name — none of whose characters is shown.
    pub masked: String,
    /// A credential of a known shape (a provider's prefix), not only a secret-looking name.
    pub known: bool,
    /// Counts the same value once across a transcript; held in memory, never shown.
    pub fp: u64,
}

/// Mask every credential in a text and say which were found. The clear text never leaves
/// this function.
pub fn mask(text: &str) -> (String, Vec<Hit>) {
    let mut hits = Vec::new();
    let mut s = shape_re()
        .replace_all(text, |c: &regex::Captures| {
            let v = &c[1];
            if is_placeholder(v) {
                v.to_string()
            } else {
                let m = bbl_hostrules::secrets::mask(v);
                hits.push(Hit {
                    masked: m.clone(),
                    known: true,
                    fp: fingerprint(v),
                });
                m
            }
        })
        .into_owned();
    s = bearer_re()
        .replace_all(&s, |c: &regex::Captures| {
            let v = &c[2];
            if v.contains('…') || !assigned_value_is_secret(v) {
                c[0].to_string()
            } else {
                let shown = format!("{} {DOTS}", &c[1]);
                hits.push(Hit {
                    masked: shown.clone(),
                    known: false,
                    fp: fingerprint(v),
                });
                shown
            }
        })
        .into_owned();
    s = assign_re()
        .replace_all(&s, |c: &regex::Captures| {
            let v = &c[3];
            if v.contains('…') || v.contains(DOTS) || !assigned_value_is_secret(v) {
                c[0].to_string()
            } else {
                let unquoted = v.trim_matches(|c| c == '"' || c == '\'');
                let (core, tail) = if unquoted.len() == v.len() {
                    split_tail(v)
                } else {
                    (unquoted, "")
                };
                hits.push(Hit {
                    masked: format!("{}={DOTS}", &c[1]),
                    known: false,
                    fp: fingerprint(core),
                });
                format!("{}{}{DOTS}{tail}", &c[1], &c[2])
            }
        })
        .into_owned();
    (s, hits)
}

/// A value under a key of a structure (`{"apiKey": "…"}`): a credential when the key says so
/// and the value is not a word, a path or a reference. A known shape is found by `mask`.
pub fn named_value(key: &str, value: &str) -> Option<Hit> {
    if !name_re().is_match(key) || shape_re().is_match(value) || !assigned_value_is_secret(value) {
        return None;
    }
    Some(Hit {
        masked: format!("{key}={DOTS}"),
        known: false,
        fp: fingerprint(value),
    })
}

/// A command shown: secrets masked, whitespace folded, cut at 400 characters.
pub fn shown(command: &str) -> String {
    let (m, _) = mask(command.trim());
    let folded: String = m.split_whitespace().collect::<Vec<_>>().join(" ");
    if folded.chars().count() > 400 {
        let cut: String = folded.chars().take(399).collect();
        format!("{cut}…")
    } else {
        folded
    }
}

// ---------------------------------------------------------------- paths

/// Resolve a path an agent named: `~` and `$HOME` against `home`, relative against `cwd`,
/// `.` and `..` folded without touching the disk.
pub fn resolve(p: &str, cwd: Option<&Path>, home: &Path) -> Option<PathBuf> {
    let p = p.trim();
    if p.is_empty() {
        return None;
    }
    let raw = if p == "~" || p == "$HOME" || p == "${HOME}" {
        home.to_path_buf()
    } else if let Some(r) = p
        .strip_prefix("~/")
        .or_else(|| p.strip_prefix("$HOME/"))
        .or_else(|| p.strip_prefix("${HOME}/"))
    {
        home.join(r)
    } else if p.starts_with('/') {
        PathBuf::from(p)
    } else {
        cwd?.join(p)
    };
    let mut out = PathBuf::new();
    for c in raw.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}

pub fn is_temp(p: &Path) -> bool {
    p.starts_with("/tmp")
        || p.starts_with("/var/tmp")
        || p.starts_with("/dev/shm")
        || p == Path::new("/dev/null")
}

/// A target whose deletion is not routine: the root, the home directory, a whole top-level
/// folder of the home (`~/Documents`), the working directory itself or everything in it, or
/// a path outside the working directory (temporary directories aside).
pub fn dangerous_target(target: &str, cwd: Option<&Path>, home: &Path) -> bool {
    let t = target.trim();
    if matches!(
        t,
        "/" | "/*"
            | "~"
            | "~/"
            | "~/*"
            | "$HOME"
            | "${HOME}"
            | "$HOME/"
            | "$HOME/*"
            | "."
            | "./"
            | "*"
            | "./*"
            | ".."
            | "../"
            | "../*"
            | ".*"
    ) {
        return true;
    }
    let Some(p) = resolve(t.trim_end_matches("/*"), cwd, home) else {
        return false;
    };
    if is_temp(&p) {
        return false;
    }
    if p == Path::new("/") || p == home {
        return true;
    }
    if p.parent() == Some(home)
        && !p
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with('.'))
    {
        return true;
    }
    if let Some(c) = cwd {
        if p == c || c.starts_with(&p) {
            return true; // the project or one of its parents
        }
        if !p.starts_with(c) {
            return true;
        }
    }
    false
}

/// For `rm`: is it recursive, and what does it name? Flags after `--` are targets.
pub fn rm_parts(tokens: &[String]) -> Option<(bool, Vec<String>)> {
    if tokens.is_empty() || program_name(&tokens[0]) != "rm" {
        return None;
    }
    let mut recursive = false;
    let mut targets = Vec::new();
    let mut dashdash = false;
    for t in &tokens[1..] {
        if !dashdash && t == "--" {
            dashdash = true;
        } else if !dashdash && t.starts_with("--") {
            if t == "--recursive" {
                recursive = true;
            }
        } else if !dashdash && t.starts_with('-') && t.len() > 1 {
            if t.contains('r') || t.contains('R') {
                recursive = true;
            }
        } else {
            targets.push(t.clone());
        }
    }
    Some((recursive, targets))
}

/// For `find … -delete`: the starting points.
pub fn find_delete_roots(tokens: &[String]) -> Option<Vec<String>> {
    if tokens.is_empty()
        || program_name(&tokens[0]) != "find"
        || !tokens.iter().any(|t| t == "-delete")
    {
        return None;
    }
    let roots: Vec<String> = tokens[1..]
        .iter()
        .take_while(|t| !t.starts_with('-') && !t.starts_with('(') && *t != "!")
        .cloned()
        .collect();
    Some(if roots.is_empty() {
        vec![".".into()]
    } else {
        roots
    })
}

/// The script a command runs, when it runs one: `bash x.sh`, `python3 tool.py`, `node a.js`,
/// `./run.sh`, `/abs/script`.
pub fn executed_script(tokens: &[String]) -> Option<String> {
    let first = tokens.first()?;
    let prog = program_name(first);
    if matches!(
        prog,
        "bash"
            | "sh"
            | "zsh"
            | "dash"
            | "python"
            | "python3"
            | "node"
            | "deno"
            | "bun"
            | "ruby"
            | "perl"
            | "pwsh"
            | "tsx"
            | "ts-node"
    ) {
        let mut i = 1;
        while i < tokens.len() && tokens[i].starts_with('-') {
            // `deno run`, `bun run` take a subcommand; `-c` and `-e` are inline code
            if matches!(tokens[i].as_str(), "-c" | "-e" | "--eval" | "-m") {
                return None;
            }
            i += 1;
        }
        if matches!(prog, "deno" | "bun") && tokens.get(i).is_some_and(|t| t == "run") {
            i += 1;
        }
        return tokens.get(i).filter(|t| looks_like_script(t)).cloned();
    }
    if (first.starts_with("./") || first.starts_with('/') || first.starts_with("~/"))
        && !first.ends_with('/')
    {
        return Some(first.clone());
    }
    None
}

fn looks_like_script(t: &str) -> bool {
    let ext = Path::new(t)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    matches!(
        ext.as_str(),
        "sh" | "bash" | "zsh" | "py" | "js" | "mjs" | "cjs" | "ts" | "rb" | "pl" | "ps1"
    ) || t.starts_with("./")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn splits_unwraps_and_strips() {
        let c = commands("cd /x && sudo -u root rm -rf build 2>/dev/null; echo done | tee log");
        assert_eq!(
            c,
            vec![
                v(&["cd", "/x"]),
                v(&["rm", "-rf", "build"]),
                v(&["echo", "done"]),
                v(&["tee", "log"])
            ]
        );
        let c = commands(r#"bash -lc "git reset --hard origin/main && npm i left-pad""#);
        assert_eq!(
            c,
            vec![
                v(&["git", "reset", "--hard", "origin/main"]),
                v(&["npm", "i", "left-pad"])
            ]
        );
        let c = commands("FOO=1 env BAR=2 timeout 30 python3 run.py # a comment");
        assert_eq!(c, vec![v(&["python3", "run.py"])]);
        assert_eq!(from_argv(&v(&["bash", "-lc", "cargo test"])), "cargo test");
    }

    #[test]
    fn masks_without_eating_hashes_or_examples() {
        let key = format!("sk-ant-api03-{}", "Zq9Lk2Vb7Xr4Tn8Mw3Pd6Hs1Fj5Gc0Ya");
        let (m, hits) = mask(&format!(
            "export ANTHROPIC_API_KEY={key} && git show 4b825dc642cb6eb9a060e54bf8d69288fbee4904"
        ));
        assert_eq!(hits.len(), 1, "{m}");
        assert!(!m.contains(&key));
        assert!(m.contains("4b825dc642cb6eb9a060e54bf8d69288fbee4904"));
        let (_, hits) = mask("OPENAI_API_KEY=sk-xxxxxxxxxxxxxxxxxxxxxxxx TOKEN=$GITHUB_TOKEN");
        assert!(hits.is_empty());
        let bearer = format!(
            "curl -H 'Authorization: Bearer {}' https://api.x",
            "abC9dEf8ghI7jkL6mnO5"
        );
        let (m, hits) = mask(&bearer);
        assert_eq!(hits.len(), 1, "{m}");
        assert!(m.contains(&format!("Bearer {DOTS}")), "{m}");
    }

    #[test]
    fn a_value_under_a_secret_name_shows_none_of_its_characters() {
        let pw = format!("{}{}", "Qm7vR2", "pL9xW4kT");
        let (m, hits) = mask(&format!("PGPASSWORD={pw} psql -h db"));
        assert_eq!(hits.len(), 1, "{m}");
        assert!(!hits[0].known);
        assert_eq!(hits[0].masked, format!("PGPASSWORD={DOTS}"));
        assert!(!m.contains(&pw[..4]), "{m}");
        assert_eq!(m, format!("PGPASSWORD={DOTS} psql -h db"));
        // inside quotes, inside a printed dict, with closing punctuation
        let (_, hits) = mask(&format!("{{'api_key': '{pw}', 'n': 1}}"));
        assert_eq!(hits.len(), 1);
        let (m, hits) = mask(&format!("token: {pw}."));
        assert_eq!(
            (hits.len(), m.ends_with(&format!("{DOTS}."))),
            (1, true),
            "{m}"
        );
        assert_eq!(
            named_value("apiKey", &pw).unwrap().masked,
            format!("apiKey={DOTS}")
        );
        assert!(named_value("model", &pw).is_none());
    }

    #[test]
    fn names_identifiers_paths_and_code_are_not_secrets() {
        for text in [
            // seen on a real machine: an identifier, Python output across lines
            "API_KEY_BY_PROVIDER_119: dict",
            "KEY: KEY_BY_PROVIDER_2024",
            "OPENAI_API_KEY=None\nHF_TOKEN=None\n[gpt-5.2]",
            "max_tokens=128000 tokenizer=bert-base-uncased-v12x author: Someone2026abc",
            "secret_key = settings.secret_key2",
            "TOKEN_FILE=src/auth/token_store2.rs",
            "api_key=os.environ.get(\"KEY1\")",
            "password: ${{ secrets.DB_PASSWORD_2 }}",
            "PWD=/home/a/proj2/build",
            "token: N/A\nHeader2025",
        ] {
            let (m, hits) = mask(text);
            assert!(hits.is_empty(), "{text} → {m}");
        }
    }

    #[test]
    fn dangerous_targets() {
        let home = Path::new("/home/a");
        let cwd = Some(Path::new("/home/a/proj"));
        for t in [
            "~/",
            "~",
            "$HOME",
            "/",
            ".",
            "*",
            "/home/a/Documents",
            "/etc/nginx",
            "..",
            "/home/a/other",
        ] {
            assert!(dangerous_target(t, cwd, home), "{t}");
        }
        for t in [
            "build",
            "target/debug",
            "/tmp/x",
            "/home/a/proj/dist",
            "node_modules",
            "/home/a/.cache/pip",
        ] {
            if t == "/home/a/.cache/pip" {
                // outside the project: dangerous by the project rule
                assert!(dangerous_target(t, cwd, home));
                continue;
            }
            assert!(!dangerous_target(t, cwd, home), "{t}");
        }
        assert_eq!(
            rm_parts(&v(&["rm", "-rf", "tests/", "~/"])),
            Some((true, v(&["tests/", "~/"])))
        );
        assert_eq!(
            executed_script(&v(&["node", "cleanup.js"])),
            Some("cleanup.js".into())
        );
        assert_eq!(executed_script(&v(&["python3", "-c", "print(1)"])), None);
        assert_eq!(
            executed_script(&v(&["./run.sh", "--fast"])),
            Some("./run.sh".into())
        );
    }
}
