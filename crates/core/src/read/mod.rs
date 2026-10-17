//! The readers: one per transcript format, each turning a host's files into actions on the
//! common schema. A reader keeps no text of the exchange: it reads the tool's name and
//! input, the result's status and code, and lets the outputs go. It also scans every line it
//! reads for credentials, keeping their masked form only.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::command;
use crate::model::{ActKind, Action, Approval, Decision, FileOp, Status};

pub mod claude;
pub mod cline;
pub mod codex;
pub mod copilot;
pub mod gemini;
pub mod goose;
pub mod kiro;
pub mod opencode;
pub mod zed;

/// What a reader needs to know.
#[derive(Debug, Clone)]
pub struct Ctx<'a> {
    pub host: &'a str,
    pub since: DateTime<Utc>,
    pub home: &'a Path,
}

/// A session as a reader found it; the scan counts its actions.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: String,
    pub project: Option<PathBuf>,
    pub transcript: PathBuf,
    pub subagent: bool,
    pub modes: Vec<String>,
    pub started: Option<DateTime<Utc>>,
    /// Kept at any age by the host, whatever its retention (Claude Desktop and Cowork
    /// sessions in Claude Code's store).
    pub exempt: bool,
}

/// A credential found in a transcript, masked.
#[derive(Debug, Clone)]
pub struct SecretHit {
    pub file: PathBuf,
    pub line: Option<u32>,
    /// The session, when one file holds many (a database).
    pub session: Option<String>,
    pub masked: String,
    /// A known shape (`sk-ant-…`), or a value under a secret-looking name.
    pub known: bool,
    /// Counts the same value once; never shown.
    pub fp: u64,
}

#[derive(Debug, Default)]
pub struct Read {
    pub sessions: Vec<SessionInfo>,
    pub actions: Vec<Action>,
    pub files: Vec<PathBuf>,
    pub secrets: Vec<SecretHit>,
    pub errors: Vec<String>,
}

impl Read {
    pub fn merge(&mut self, o: Read) {
        self.sessions.extend(o.sessions);
        self.actions.extend(o.actions);
        self.files.extend(o.files);
        self.secrets.extend(o.secrets);
        self.errors.extend(o.errors);
    }

    /// Scan a piece of text for credentials (masked forms only are kept). JSON is read as
    /// what it says — its strings unescaped, a value beside its key — so an escaped line
    /// break or a quote never glues two values into one.
    pub fn scan(&mut self, file: &Path, line: Option<u32>, text: &str) {
        self.scan_in(file, line, None, text);
    }

    pub fn scan_in(&mut self, file: &Path, line: Option<u32>, session: Option<&str>, text: &str) {
        match serde_json::from_str::<Value>(text) {
            Ok(v) => self.scan_value(file, line, session, &v),
            Err(_) => self.keep(file, line, session, command::mask(text).1),
        }
    }

    /// Scan a JSON value already parsed by the reader.
    pub fn scan_value(&mut self, file: &Path, line: Option<u32>, session: Option<&str>, v: &Value) {
        let mut hits = Vec::new();
        walk_strings(v, None, &mut hits);
        self.keep(file, line, session, hits);
    }

    fn keep(
        &mut self,
        file: &Path,
        line: Option<u32>,
        session: Option<&str>,
        hits: Vec<command::Hit>,
    ) {
        for h in hits {
            self.secrets.push(SecretHit {
                file: file.to_path_buf(),
                line,
                session: session.map(String::from),
                masked: h.masked,
                known: h.known,
                fp: h.fp,
            });
        }
    }

    /// Note a session once.
    pub fn session(&mut self, s: SessionInfo) {
        if let Some(e) = self
            .sessions
            .iter_mut()
            .find(|x| x.id == s.id && x.transcript == s.transcript)
        {
            if e.project.is_none() {
                e.project = s.project;
            }
            for m in s.modes {
                if !e.modes.contains(&m) {
                    e.modes.push(m);
                }
            }
            if e.started.is_none() || s.started.is_some_and(|t| e.started.is_some_and(|x| t < x)) {
                e.started = s.started.or(e.started);
            }
        } else {
            self.sessions.push(s);
        }
    }
}

/// Dispatch on the sheet's reader name.
pub fn read(reader: &str, ctx: &Ctx, files: &[PathBuf]) -> Read {
    match reader {
        "claude-jsonl" => claude::read(ctx, files),
        "codex-jsonl" => codex::read(ctx, files),
        "gemini-jsonl" => gemini::read_gemini(ctx, files),
        "qwen-jsonl" => gemini::read_qwen(ctx, files),
        "opencode-sqlite" => opencode::read(ctx, files),
        "goose-sqlite" => goose::read(ctx, files),
        "cline-tasks" => cline::read_tasks(ctx, files),
        "cline-sessions" => cline::read_sessions(ctx, files),
        "copilot-jsonl" => copilot::read(ctx, files),
        "zed-sqlite" => zed::read(ctx, files),
        "kiro-sqlite" => kiro::read(ctx, files),
        other => Read {
            errors: vec![format!("unknown reader {other}")],
            ..Default::default()
        },
    }
}

// ---------------------------------------------------------------- helpers

/// A new action with nothing known yet but where it comes from.
pub fn action(ctx: &Ctx, session: &str, tool: &str, source: &Path, line: Option<u32>) -> Action {
    Action {
        host: ctx.host.to_string(),
        session: session.to_string(),
        call_id: None,
        tool: tool.to_string(),
        kind: ActKind::Other,
        command: None,
        path: None,
        op: None,
        inferred: false,
        server: None,
        diff: None,
        when: None,
        cwd: None,
        approval: Approval::not_recorded(),
        status: Status::Unknown,
        exit_code: None,
        model: None,
        subagent: false,
        source: source.to_path_buf(),
        line,
    }
}

fn norm(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect::<String>()
        .to_lowercase()
}

const SHELL: &[&str] = &[
    "bash",
    "shell",
    "terminal",
    "runshellcommand",
    "executecommand",
    "execcommand",
    "executebash",
    "runcommand",
    "runterminalcmd",
    "runinterminal",
    "powershell",
    "localshell",
    "shellcommand",
    "command",
    "runcommands",
    "exec",
];
const DELETE: &[&str] = &[
    "deletefile",
    "deletepath",
    "filedeleted",
    "removefile",
    "delete",
    "rm",
];
const CREATE: &[&str] = &[
    "write",
    "writefile",
    "create",
    "createfile",
    "newfilecreated",
    "writetofile",
];
const MODIFY: &[&str] = &[
    "edit",
    "multiedit",
    "replace",
    "strreplace",
    "applypatch",
    "patch",
    "editedexistingfile",
    "applieddiff",
    "notebookedit",
    "editfile",
    "insertedit",
    "insertedittofile",
    "searchreplace",
    "applydiff",
    "insertcontent",
    "searchandreplace",
    "movepath",
    "copypath",
    "createdirectory",
];
const READ: &[&str] = &[
    "read",
    "readfile",
    "view",
    "glob",
    "grep",
    "ls",
    "listfiles",
    "listfilestoplevel",
    "listfilesrecursive",
    "searchfiles",
    "findpath",
    "listdirectory",
    "readmanyfiles",
    "codebasesearch",
    "fsread",
    "searchfilecontent",
    "readcommandoutput",
    "diagnostics",
    "open",
];
const NETWORK: &[&str] = &[
    "webfetch",
    "websearch",
    "googlewebsearch",
    "fetch",
    "browseraction",
    "browseractionlaunch",
    "httprequest",
    "websearchcall",
    "webfetchtool",
];
const AGENT: &[&str] = &[
    "task",
    "agent",
    "newtask",
    "spawnagent",
    "subagent",
    "dispatchagent",
];

/// The command an input names: `command` (string or argv), `cmd`, `script`.
pub fn command_of(input: &Value) -> Option<String> {
    for k in ["command", "cmd", "script", "commandLine", "fullCommandText"] {
        match input.get(k) {
            Some(Value::String(s)) if !s.trim().is_empty() => return Some(s.clone()),
            Some(Value::Array(a)) if !a.is_empty() => {
                let argv: Vec<String> = a
                    .iter()
                    .filter_map(|x| x.as_str().map(String::from))
                    .collect();
                if !argv.is_empty() {
                    return Some(command::from_argv(&argv));
                }
            }
            _ => {}
        }
    }
    None
}

/// The path an input names.
pub fn path_of(input: &Value) -> Option<String> {
    for k in [
        "file_path",
        "filePath",
        "path",
        "target_file",
        "notebook_path",
        "input_path",
        "absolute_path",
        "filename",
        "file",
        "abs_path",
    ] {
        if let Some(s) = input.get(k).and_then(Value::as_str) {
            if !s.is_empty() {
                return Some(s.to_string());
            }
        }
    }
    None
}

/// Classify a tool call by its name and input: shell command, file and what happens to it,
/// MCP, network, sub-agent. `cwd` resolves relative paths.
pub fn classify(a: &mut Action, input: &Value, cwd: Option<&Path>, home: &Path) {
    let raw = a.tool.clone();
    if let Some(rest) = raw.strip_prefix("mcp__") {
        a.kind = ActKind::Mcp;
        a.server = rest.split("__").next().map(String::from);
        return;
    }
    let n = norm(&raw);
    let cmd = command_of(input);
    let path =
        path_of(input).and_then(|p| command::resolve(&p, cwd, home).or(Some(PathBuf::from(p))));
    // editors driven by a verb (`text_editor`, `fs_write`, `str_replace_editor`)
    if n.contains("texteditor")
        || n == "fswrite"
        || n == "strreplaceeditor"
        || n == "strreplacebasededittool"
    {
        let verb = input.get("command").and_then(Value::as_str).unwrap_or("");
        a.kind = ActKind::File;
        a.path = path;
        a.op = Some(match verb {
            "create" | "write" => FileOp::Create,
            "view" | "read" => FileOp::Read,
            _ => FileOp::Modify,
        });
        return;
    }
    if SHELL.contains(&n.as_str()) && cmd.is_some() {
        a.kind = ActKind::Shell;
        a.command = cmd;
        return;
    }
    let op = if DELETE.contains(&n.as_str()) {
        Some(FileOp::Delete)
    } else if CREATE.contains(&n.as_str()) {
        Some(FileOp::Create)
    } else if MODIFY.contains(&n.as_str()) {
        Some(FileOp::Modify)
    } else if READ.contains(&n.as_str()) {
        Some(FileOp::Read)
    } else {
        None
    };
    if let Some(op) = op {
        a.kind = ActKind::File;
        a.op = Some(op);
        a.path = path;
        return;
    }
    if NETWORK.contains(&n.as_str()) {
        a.kind = ActKind::Network;
        return;
    }
    if AGENT.contains(&n.as_str()) {
        a.kind = ActKind::Agent;
        return;
    }
    if cmd.is_some() {
        a.kind = ActKind::Shell;
        a.command = cmd;
        return;
    }
    a.path = path;
}

/// Lines added and removed in a unified diff.
pub fn diff_counts(diff: &str) -> (u32, u32) {
    let mut add = 0;
    let mut del = 0;
    for l in diff.lines() {
        if l.starts_with("+++") || l.starts_with("---") {
            continue;
        }
        if l.starts_with('+') {
            add += 1;
        } else if l.starts_with('-') {
            del += 1;
        }
    }
    (add, del)
}

/// `Exit code: 3`, `Exit Code: 3`, `failed with exit code 3`.
pub fn exit_code_in(text: &str) -> Option<i64> {
    let lower = text.to_lowercase();
    let i = lower.find("exit code")?;
    let rest = &text[i + "exit code".len()..];
    let digits: String = rest
        .trim_start_matches([':', ' '])
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '-')
        .collect();
    digits.parse().ok()
}

/// Any timestamp: RFC 3339, epoch seconds or milliseconds.
pub fn time(v: Option<&Value>) -> Option<DateTime<Utc>> {
    match v? {
        Value::String(s) => DateTime::parse_from_rfc3339(s)
            .ok()
            .map(|t| t.with_timezone(&Utc))
            .or_else(|| s.parse::<i64>().ok().and_then(epoch)),
        Value::Number(n) => n.as_f64().and_then(|f| epoch(f as i64)),
        _ => None,
    }
}

pub fn epoch(n: i64) -> Option<DateTime<Utc>> {
    if n > 100_000_000_000 {
        DateTime::<Utc>::from_timestamp_millis(n)
    } else {
        DateTime::<Utc>::from_timestamp(n, 0)
    }
}

/// The file's modification time.
pub fn mtime(p: &Path) -> Option<DateTime<Utc>> {
    std::fs::metadata(p)
        .ok()?
        .modified()
        .ok()
        .map(DateTime::<Utc>::from)
}

/// Read a text file, decompressing zstd by extension.
pub fn read_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if path.extension().is_some_and(|e| e == "zst") {
        let out =
            zstd::decode_all(&bytes[..]).map_err(|e| format!("{}: zstd: {e}", path.display()))?;
        Ok(String::from_utf8_lossy(&out).into_owned())
    } else {
        Ok(String::from_utf8_lossy(&bytes).into_owned())
    }
}

/// Open a SQLite database read-only and immutable: nothing is written, not even a `-wal`
/// or `-shm` file (the price: changes not yet checkpointed by the host are not seen).
pub fn open_ro(path: &Path) -> Result<rusqlite::Connection, String> {
    let uri = format!("file:{}?mode=ro&immutable=1", path.display());
    rusqlite::Connection::open_with_flags(
        uri,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
            | rusqlite::OpenFlags::SQLITE_OPEN_URI
            | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| e.to_string())
}

pub fn tables(conn: &rusqlite::Connection) -> Vec<String> {
    conn.prepare("SELECT name FROM sqlite_master WHERE type='table'")
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(0))
                .map(|it| it.flatten().collect())
        })
        .unwrap_or_default()
}

pub fn columns(conn: &rusqlite::Connection, table: &str) -> Vec<String> {
    conn.prepare(&format!("PRAGMA table_info({table})"))
        .and_then(|mut s| {
            s.query_map([], |r| r.get::<_, String>(1))
                .map(|it| it.flatten().collect())
        })
        .unwrap_or_default()
}

/// The text of a tool result, whatever its shape (string, `[{type, text}]`, `{content}`),
/// for the reader to test against a host's fixed message — never kept.
pub fn result_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(a) => a.iter().map(result_text).collect::<Vec<_>>().join("\n"),
        Value::Object(o) => {
            for k in [
                "text", "content", "output", "Text", "body", "message", "error",
            ] {
                if let Some(x) = o.get(k) {
                    return result_text(x);
                }
            }
            // a wrapper (`functionResponse`, `response`): look inside
            o.values()
                .map(result_text)
                .filter(|t| !t.is_empty())
                .collect::<Vec<_>>()
                .join("\n")
        }
        _ => String::new(),
    }
}

/// Mark an action denied, as the host recorded it.
pub fn deny(a: &mut Action, source: Option<&str>) {
    a.status = Status::Denied;
    a.approval.decision = Decision::Denied;
    if let Some(s) = source {
        a.approval.source = Some(s.to_string());
    }
}

/// Index of call ids to actions, for results that come after their call.
#[derive(Default)]
pub struct Calls(HashMap<String, Vec<usize>>);

impl Calls {
    pub fn add(&mut self, id: &str, i: usize) {
        self.0.entry(id.to_string()).or_default().push(i);
    }
    pub fn get(&self, id: &str) -> &[usize] {
        self.0.get(id).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Every string of a JSON value, scanned; a string under a secret-looking key is also
/// judged as that key's value.
fn walk_strings(v: &Value, key: Option<&str>, hits: &mut Vec<command::Hit>) {
    match v {
        Value::String(s) => {
            hits.extend(command::mask(s).1);
            if let Some(h) = key.and_then(|k| command::named_value(k, s)) {
                hits.push(h);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk_strings(x, None, hits)),
        Value::Object(o) => o.iter().for_each(|(k, x)| walk_strings(x, Some(k), hits)),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> (PathBuf, DateTime<Utc>) {
        (PathBuf::from("/home/a"), Utc::now())
    }

    #[test]
    fn classifies_by_name_and_input() {
        let (home, since) = ctx();
        let c = Ctx {
            host: "x",
            since,
            home: &home,
        };
        let cwd = Some(Path::new("/home/a/p"));
        let mut a = action(&c, "s", "Bash", Path::new("/t"), None);
        classify(&mut a, &json!({"command": "ls"}), cwd, &home);
        assert_eq!((a.kind, a.command.as_deref()), (ActKind::Shell, Some("ls")));
        let mut a = action(&c, "s", "write_file", Path::new("/t"), None);
        classify(&mut a, &json!({"file_path": "src/a.rs"}), cwd, &home);
        assert_eq!(
            (a.kind, a.op, a.path.as_deref()),
            (
                ActKind::File,
                Some(FileOp::Create),
                Some(Path::new("/home/a/p/src/a.rs"))
            )
        );
        let mut a = action(&c, "s", "mcp__github__create_issue", Path::new("/t"), None);
        classify(&mut a, &json!({}), cwd, &home);
        assert_eq!(
            (a.kind, a.server.as_deref()),
            (ActKind::Mcp, Some("github"))
        );
        let mut a = action(&c, "s", "developer__text_editor", Path::new("/t"), None);
        classify(
            &mut a,
            &json!({"command": "view", "path": "/x"}),
            cwd,
            &home,
        );
        assert_eq!(a.op, Some(FileOp::Read));
        let mut a = action(&c, "s", "shell", Path::new("/t"), None);
        classify(
            &mut a,
            &json!({"command": ["bash", "-lc", "cargo test"]}),
            cwd,
            &home,
        );
        assert_eq!(a.command.as_deref(), Some("cargo test"));
        assert_eq!(exit_code_in("Exit code: 2\nWall time"), Some(2));
        assert_eq!(
            exit_code_in("Command \"x\" failed with exit code 127."),
            Some(127)
        );
        assert_eq!(diff_counts("--- a\n+++ b\n@@\n-x\n+y\n+z\n"), (2, 1));
    }
}
