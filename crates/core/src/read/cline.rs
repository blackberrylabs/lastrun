//! Cline and Roo Code: one folder per task with `ui_messages.json` — an array of
//! `{ts, type: ask|say, ask|say, text}`. `command` carries the command (an `ask` waits for
//! approval, a `say` ran without asking), `tool` carries a JSON description of a file
//! operation, `use_mcp_server` a tool of an MCP server; `command_permission_denied` is
//! Cline's mark of a refusal. Cline's SDK runtime writes `<id>.messages.json` with
//! `tool-call` / `tool-result` parts, and a manifest `<id>.json` with the working directory.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{action, classify, deny, epoch, mtime, read_text, time, Calls, Ctx, Read, SessionInfo};
use crate::model::{ActKind, Action, Decision, FileOp, Status};

pub fn read_tasks(ctx: &Ctx, files: &[PathBuf]) -> Read {
    let mut out = Read::default();
    for f in files {
        if mtime(f).is_some_and(|t| t < ctx.since) {
            continue;
        }
        match read_text(f) {
            Ok(text) => {
                out.files.push(f.clone());
                out.scan(f, None, &text);
                task(ctx, f, &text, &mut out);
            }
            Err(e) => out.errors.push(e),
        }
    }
    out
}

fn tool_op(tool: &str) -> Option<FileOp> {
    match tool {
        "newFileCreated" => Some(FileOp::Create),
        "fileDeleted" => Some(FileOp::Delete),
        "editedExistingFile" | "appliedDiff" | "insertContent" | "searchAndReplace" => {
            Some(FileOp::Modify)
        }
        "readFile"
        | "listFilesTopLevel"
        | "listFilesRecursive"
        | "searchFiles"
        | "listCodeDefinitionNames"
        | "codebaseSearch" => Some(FileOp::Read),
        _ => None,
    }
}

fn task(ctx: &Ctx, f: &Path, text: &str, out: &mut Read) {
    let session = f
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Ok(Value::Array(msgs)) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let mut started = None;
    // the last command still waiting for its output or its refusal
    let mut open_cmd: Option<usize> = None;
    let mut n = 0;
    for m in &msgs {
        let when = m.get("ts").and_then(Value::as_i64).and_then(epoch);
        if started.is_none() {
            started = when;
        }
        if when.is_some_and(|w| w < ctx.since)
            || m.get("partial").and_then(Value::as_bool) == Some(true)
        {
            continue;
        }
        let kind = m.get("type").and_then(Value::as_str).unwrap_or("");
        let what = m.get(kind).and_then(Value::as_str).unwrap_or("");
        let body = m.get("text").and_then(Value::as_str).unwrap_or("");
        match what {
            "command" => {
                let mut a = action(ctx, &session, "execute_command", f, None);
                a.kind = ActKind::Shell;
                a.command = Some(body.to_string());
                a.when = when;
                if kind == "say" {
                    a.approval.decision = Decision::Allowed;
                    a.approval.source = Some("auto-approval".into());
                    a.status = Status::Completed;
                }
                open_cmd = Some(out.actions.len());
                out.actions.push(a);
                n += 1;
            }
            "command_output" => {
                if let Some(i) = open_cmd {
                    if out.actions[i].status == Status::Unknown {
                        out.actions[i].status = Status::Completed;
                    }
                }
            }
            "command_permission_denied" => {
                if let Some(i) = open_cmd.take() {
                    deny(&mut out.actions[i], None);
                }
            }
            "tool" => {
                let Ok(t) = serde_json::from_str::<Value>(body) else {
                    continue;
                };
                let tool = t.get("tool").and_then(Value::as_str).unwrap_or("?");
                let mut a = action(ctx, &session, tool, f, None);
                a.when = when;
                if let Some(op) = tool_op(tool) {
                    a.kind = ActKind::File;
                    a.op = Some(op);
                    a.path = t.get("path").and_then(Value::as_str).map(PathBuf::from);
                } else {
                    classify(&mut a, &t, None, ctx.home);
                }
                if let Some(s) = t.get("diffStats") {
                    if let (Some(x), Some(y)) = (
                        s.get("added").and_then(Value::as_u64),
                        s.get("removed").and_then(Value::as_u64),
                    ) {
                        a.diff = Some((x as u32, y as u32));
                    }
                }
                if kind == "say" {
                    a.status = Status::Completed;
                }
                out.actions.push(a);
                n += 1;
            }
            "use_mcp_server" => {
                let t: Value = serde_json::from_str(body).unwrap_or(Value::Null);
                let mut a = action(
                    ctx,
                    &session,
                    t.get("toolName").and_then(Value::as_str).unwrap_or("mcp"),
                    f,
                    None,
                );
                a.kind = ActKind::Mcp;
                a.server = t
                    .get("serverName")
                    .and_then(Value::as_str)
                    .map(String::from);
                a.when = when;
                out.actions.push(a);
                n += 1;
            }
            _ => {}
        }
    }
    if n > 0 {
        out.session(SessionInfo {
            id: session,
            project: None,
            transcript: f.to_path_buf(),
            subagent: false,
            modes: Vec::new(),
            started,
            exempt: false,
        });
    }
}

pub fn read_sessions(ctx: &Ctx, files: &[PathBuf]) -> Read {
    let mut out = Read::default();
    for f in files {
        if mtime(f).is_some_and(|t| t < ctx.since) {
            continue;
        }
        match read_text(f) {
            Ok(text) => {
                out.files.push(f.clone());
                out.scan(f, None, &text);
                sdk(ctx, f, &text, &mut out);
            }
            Err(e) => out.errors.push(e),
        }
    }
    out
}

fn sdk(ctx: &Ctx, f: &Path, text: &str, out: &mut Read) {
    let name = f
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let session = name.trim_end_matches(".messages.json").to_string();
    let manifest: Value = f
        .parent()
        .map(|d| d.join(format!("{session}.json")))
        .and_then(|m| std::fs::read_to_string(m).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Value::Null);
    let cwd = manifest
        .get("cwd")
        .or_else(|| manifest.get("workspace_root"))
        .and_then(Value::as_str)
        .map(PathBuf::from);
    let subagent = manifest
        .get("is_subagent")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let msgs = v
        .as_array()
        .cloned()
        .or_else(|| v.get("messages").and_then(Value::as_array).cloned())
        .unwrap_or_default();
    let mut calls = Calls::default();
    let mut started = None;
    for m in msgs {
        let when = time(m.get("createdAt"));
        if started.is_none() {
            started = when;
        }
        if when.is_some_and(|w| w < ctx.since) {
            continue;
        }
        let model = m
            .pointer("/modelInfo/id")
            .and_then(Value::as_str)
            .map(String::from);
        for p in m
            .get("content")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            match p.get("type").and_then(Value::as_str) {
                Some("tool-call") => {
                    let mut a: Action = action(
                        ctx,
                        &session,
                        p.get("toolName").and_then(Value::as_str).unwrap_or("?"),
                        f,
                        None,
                    );
                    a.when = when;
                    a.cwd = cwd.clone();
                    a.model = model.clone();
                    a.subagent = subagent;
                    classify(
                        &mut a,
                        p.get("input").unwrap_or(&Value::Null),
                        cwd.as_deref(),
                        ctx.home,
                    );
                    if let Some(id) = p.get("toolCallId").and_then(Value::as_str) {
                        a.call_id = Some(id.to_string());
                        calls.add(id, out.actions.len());
                    }
                    out.actions.push(a);
                }
                Some("tool-result") => {
                    let Some(id) = p.get("toolCallId").and_then(Value::as_str) else {
                        continue;
                    };
                    let err = p.get("isError").and_then(Value::as_bool).unwrap_or(false);
                    for &i in calls.get(id) {
                        out.actions[i].status = if err {
                            Status::Failed
                        } else {
                            Status::Completed
                        };
                    }
                }
                _ => {}
            }
        }
    }
    out.session(SessionInfo {
        id: session,
        project: cwd,
        transcript: f.to_path_buf(),
        subagent,
        modes: Vec::new(),
        started,
        exempt: false,
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn task_commands_tools_and_refusals() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("tasks/t1");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("ui_messages.json");
        let now = Utc::now().timestamp_millis();
        let text = format!(
            r#"[{{"ts":{now},"type":"ask","ask":"command","text":"npm install axios"}},
                {{"ts":{now},"type":"say","say":"command_output","text":"added 1 package"}},
                {{"ts":{now},"type":"ask","ask":"command","text":"rm -rf ~/"}},
                {{"ts":{now},"type":"say","say":"command_permission_denied","text":""}},
                {{"ts":{now},"type":"say","say":"tool","text":"{{\"tool\":\"fileDeleted\",\"path\":\"src/old.ts\"}}"}}]"#
        );
        std::fs::write(&f, text).unwrap();
        let home = PathBuf::from("/home/a");
        let ctx = Ctx {
            host: "cline",
            since: Utc::now() - chrono::Duration::days(30),
            home: &home,
        };
        let r = read_tasks(&ctx, &[f]);
        assert_eq!(r.actions.len(), 3);
        assert_eq!(r.actions[0].status, Status::Completed);
        assert_eq!(r.actions[1].approval.decision, Decision::Denied);
        assert_eq!(
            (r.actions[2].op, r.actions[2].path.as_deref()),
            (Some(FileOp::Delete), Some(Path::new("src/old.ts")))
        );
        assert_eq!(r.sessions[0].id, "t1");
    }
}
