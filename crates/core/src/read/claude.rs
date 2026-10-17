//! Claude Code: one JSONL per session under `~/.claude/projects/<project>/`, sub-agents under
//! `<session>/subagents/`. `tool_use` blocks in assistant lines are the calls; `tool_result`
//! blocks in user lines, with the out-of-model `toolUseResult`, are the results; the result
//! line carries `permissionDecision` and `permissionMode` when they apply.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    action, classify, deny, diff_counts, mtime, read_text, result_text, time, Calls, Ctx, Read,
    SessionInfo,
};
use crate::model::{ActKind, Decision, FileOp, Status};

/// The text Claude Code sends back to the model when the person declines a tool use
/// (claudeissues #14967, report C of the cadrage).
const DECLINED: &str = "The user doesn't want to proceed";
const INTERRUPTED: &str = "[Request interrupted by user";

pub fn read(ctx: &Ctx, files: &[PathBuf]) -> Read {
    let mut out = Read::default();
    for f in files {
        if mtime(f).is_some_and(|t| t < ctx.since) {
            continue;
        }
        match read_text(f) {
            Ok(text) => {
                out.files.push(f.clone());
                one(ctx, f, &text, &mut out);
            }
            Err(e) => out.errors.push(e),
        }
    }
    out
}

fn one(ctx: &Ctx, f: &Path, text: &str, out: &mut Read) {
    let subagent_file = f.components().any(|c| c.as_os_str() == "subagents");
    let stem = f
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let start = out.actions.len();
    let mut calls = Calls::default();
    let mut mode: Option<String> = None;
    let mut session_id: Option<String> = None;
    let mut project = None;
    let mut started = None;
    // sessions started or continued in Claude Desktop or Cowork are kept at any age
    let mut exempt = false;
    for (n, line) in text.lines().enumerate() {
        let lineno = Some((n + 1) as u32);
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            out.scan(f, lineno, line);
            continue;
        };
        out.scan_value(f, lineno, None, &v);
        if session_id.is_none() {
            session_id = v.get("sessionId").and_then(Value::as_str).map(String::from);
        }
        if project.is_none() {
            project = v.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        }
        if v.get("entrypoint")
            .and_then(Value::as_str)
            .is_some_and(|e| e.contains("cowork") || e.contains("desktop"))
        {
            exempt = true;
        }
        let when = time(v.get("timestamp"));
        if started.is_none() {
            started = when;
        }
        if let Some(m) = v.get("permissionMode").and_then(Value::as_str) {
            mode = Some(m.to_string());
        }
        let cwd = v.get("cwd").and_then(Value::as_str).map(PathBuf::from);
        let sidechain = subagent_file
            || v.get("isSidechain")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        let session = session_id.clone().unwrap_or_else(|| stem.clone());
        let Some(content) = v.pointer("/message/content").and_then(Value::as_array) else {
            continue;
        };
        match v.get("type").and_then(Value::as_str) {
            Some("assistant") => {
                for b in content {
                    if b.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let name = b.get("name").and_then(Value::as_str).unwrap_or("?");
                    let mut a = action(ctx, &session, name, f, lineno);
                    a.call_id = b.get("id").and_then(Value::as_str).map(String::from);
                    a.when = when;
                    a.cwd = cwd.clone();
                    a.model = v
                        .pointer("/message/model")
                        .and_then(Value::as_str)
                        .map(String::from);
                    a.subagent = sidechain;
                    a.approval.mode = mode.clone();
                    let input = b.get("input").cloned().unwrap_or(Value::Null);
                    classify(&mut a, &input, cwd.as_deref(), ctx.home);
                    if let Some(id) = &a.call_id {
                        calls.add(id, out.actions.len());
                    }
                    out.actions.push(a);
                }
            }
            Some("user") => {
                // `toolUseResult` describes the line's result; it is only trusted when the
                // line carries one result, as Claude Code writes them
                let results = content
                    .iter()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_result"))
                    .count();
                for b in content {
                    if b.get("type").and_then(Value::as_str) != Some("tool_result") {
                        continue;
                    }
                    let Some(id) = b.get("tool_use_id").and_then(Value::as_str) else {
                        continue;
                    };
                    for &i in calls.get(id) {
                        let a = &mut out.actions[i];
                        let res = result_text(b.get("content").unwrap_or(&Value::Null));
                        let is_error = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        a.status = if res.starts_with(DECLINED) {
                            deny(a, Some("user"));
                            Status::Denied
                        } else if res.starts_with(INTERRUPTED) {
                            Status::Interrupted
                        } else if is_error {
                            Status::Failed
                        } else {
                            Status::Completed
                        };
                        if let Some(m) = &mode {
                            a.approval.mode = Some(m.clone());
                        }
                        if let Some(d) = v.get("permissionDecision") {
                            match d.get("decision").and_then(Value::as_str) {
                                Some("accept" | "allow") => a.approval.decision = Decision::Allowed,
                                Some("deny" | "reject") => deny(a, None),
                                _ => {}
                            }
                            a.approval.source =
                                d.get("source").and_then(Value::as_str).map(String::from);
                        }
                        if let Some(r) = v.get("toolUseResult").filter(|_| results == 1) {
                            if r.get("interrupted").and_then(Value::as_bool) == Some(true) {
                                a.status = Status::Interrupted;
                            }
                            if a.kind == ActKind::File {
                                if let Some(p) = r.get("filePath").and_then(Value::as_str) {
                                    a.path = Some(PathBuf::from(p));
                                }
                                match r.get("type").and_then(Value::as_str) {
                                    Some("create") => a.op = Some(FileOp::Create),
                                    Some("update") => a.op = Some(FileOp::Modify),
                                    _ => {}
                                }
                                if let Some(hunks) = r
                                    .get("structuredPatch")
                                    .and_then(Value::as_array)
                                    .filter(|h| !h.is_empty())
                                {
                                    let mut add = 0;
                                    let mut del = 0;
                                    for h in hunks {
                                        let lines: Vec<&str> = h
                                            .get("lines")
                                            .and_then(Value::as_array)
                                            .map(|l| l.iter().filter_map(Value::as_str).collect())
                                            .unwrap_or_default();
                                        let (x, y) = diff_counts(&lines.join("\n"));
                                        add += x;
                                        del += y;
                                    }
                                    a.diff = Some((add, del));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let session = session_id.unwrap_or(stem);
    // a sub-agent file names its parent session in `sessionId`
    for a in &mut out.actions[start..] {
        a.session = session.clone();
    }
    let modes: Vec<String> = out.actions[start..]
        .iter()
        .filter_map(|a| a.approval.mode.clone())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    out.session(SessionInfo {
        id: session,
        project,
        transcript: if subagent_file {
            parent_transcript(f)
        } else {
            f.to_path_buf()
        },
        subagent: false,
        modes,
        started,
        exempt,
    });
}

/// `<project>/<session>/subagents/agent-x.jsonl` → `<project>/<session>.jsonl`.
fn parent_transcript(f: &Path) -> PathBuf {
    let mut p = f.to_path_buf();
    while p.file_name().is_some_and(|n| n != "subagents") {
        if !p.pop() {
            return f.to_path_buf();
        }
    }
    p.pop(); // subagents
    let session_dir = p.clone();
    p.pop();
    p.join(format!(
        "{}.jsonl",
        session_dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn calls_results_decisions_and_refusals() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("s1.jsonl");
        let lines = [
            r#"{"type":"user","sessionId":"s1","cwd":"/home/a/p","timestamp":"2026-10-17T10:00:00Z","permissionMode":"bypassPermissions","message":{"content":"go"}}"#,
            r#"{"type":"assistant","sessionId":"s1","cwd":"/home/a/p","timestamp":"2026-10-17T10:00:01Z","message":{"model":"m","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"rm -rf build"}},{"type":"tool_use","id":"t2","name":"Edit","input":{"file_path":"/home/a/p/a.rs"}}]}}"#,
            r#"{"type":"user","sessionId":"s1","timestamp":"2026-10-17T10:00:02Z","permissionDecision":{"decision":"accept","source":"config"},"message":{"content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]},"toolUseResult":{"stdout":"x","stderr":"","interrupted":false}}"#,
            r#"{"type":"user","sessionId":"s1","timestamp":"2026-10-17T10:00:03Z","message":{"content":[{"type":"tool_result","tool_use_id":"t2","is_error":true,"content":[{"type":"text","text":"The user doesn't want to proceed with this tool use."}]}]}}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        let home = PathBuf::from("/home/a");
        let ctx = Ctx {
            host: "claude-code",
            since: Utc::now() - chrono::Duration::days(3650),
            home: &home,
        };
        let r = read(&ctx, &[f]);
        assert_eq!(r.actions.len(), 2);
        let a = &r.actions[0];
        assert_eq!(
            (a.command.as_deref(), a.status, a.approval.decision),
            (Some("rm -rf build"), Status::Completed, Decision::Allowed)
        );
        assert_eq!(a.approval.mode.as_deref(), Some("bypassPermissions"));
        assert_eq!(a.approval.source.as_deref(), Some("config"));
        let b = &r.actions[1];
        assert_eq!(
            (b.status, b.approval.decision),
            (Status::Denied, Decision::Denied)
        );
        assert_eq!(
            r.sessions[0].project.as_deref(),
            Some(Path::new("/home/a/p"))
        );
    }

    #[test]
    fn subagent_files_belong_to_their_parent() {
        assert_eq!(
            parent_transcript(Path::new(
                "/h/.claude/projects/-p/s1/subagents/agent-1.jsonl"
            )),
            PathBuf::from("/h/.claude/projects/-p/s1.jsonl")
        );
    }
}
