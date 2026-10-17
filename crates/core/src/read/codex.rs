//! Codex CLI: one rollout per session, `{timestamp, type, payload}` per line, `.jsonl.zst`
//! after seven days. Calls are `function_call` / `local_shell_call` / `custom_tool_call`
//! (apply_patch) response items; results are `function_call_output` with "Exit code: N" in
//! the text; `patch_apply_end` and `mcp_tool_call_end` events, and in the paginated history
//! mode `item_completed` with `command_execution` (status `declined`) and `file_change`.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    action, classify, command_of, deny, diff_counts, exit_code_in, mtime, read_text, result_text,
    time, Calls, Ctx, Read, SessionInfo,
};
use crate::command;
use crate::model::{ActKind, Action, Decision, FileOp, Status};

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

/// `rollout-2026-10-17T10-00-00-<uuid>.jsonl[.zst]` → the uuid.
fn id_from_name(f: &Path) -> String {
    let name = f
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let base = name.trim_end_matches(".zst").trim_end_matches(".jsonl");
    if base.len() >= 36 {
        base[base.len() - 36..].to_string()
    } else {
        base.to_string()
    }
}

struct Turn {
    cwd: Option<PathBuf>,
    mode: Option<String>,
    model: Option<String>,
}

fn sandbox_mode(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o
            .get("mode")
            .or_else(|| o.get("type"))
            .and_then(Value::as_str)
            .map(String::from),
        _ => None,
    }
}

fn one(ctx: &Ctx, f: &Path, text: &str, out: &mut Read) {
    let mut session = id_from_name(f);
    let mut project: Option<PathBuf> = None;
    let mut started = None;
    let mut subagent = false;
    let mut modes: Vec<String> = Vec::new();
    let mut turn = Turn {
        cwd: None,
        mode: None,
        model: None,
    };
    let mut calls = Calls::default();
    let start = out.actions.len();
    for (n, line) in text.lines().enumerate() {
        let lineno = Some((n + 1) as u32);
        out.scan(f, lineno, line);
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let when = time(v.get("timestamp"));
        if started.is_none() {
            started = when;
        }
        let kind = v.get("type").and_then(Value::as_str).unwrap_or("");
        let p = v.get("payload").cloned().unwrap_or_else(|| v.clone());
        let ptype = p.get("type").and_then(Value::as_str).unwrap_or("");
        match (kind, ptype) {
            ("session_meta", _) => {
                if let Some(id) = p
                    .get("id")
                    .or_else(|| p.get("session_id"))
                    .and_then(Value::as_str)
                {
                    session = id.to_string();
                }
                project = p.get("cwd").and_then(Value::as_str).map(PathBuf::from);
                subagent = p.get("parent_thread_id").is_some_and(|x| !x.is_null())
                    || p.get("agent_role").is_some_and(|x| !x.is_null());
                turn.cwd = project.clone();
            }
            ("turn_context", _) => {
                if let Some(c) = p.get("cwd").and_then(Value::as_str) {
                    turn.cwd = Some(PathBuf::from(c));
                }
                let sandbox = sandbox_mode(p.get("sandbox_policy"));
                let approval = p
                    .get("approval_policy")
                    .and_then(Value::as_str)
                    .map(String::from);
                for m in [&sandbox, &approval].into_iter().flatten() {
                    if !modes.contains(m) {
                        modes.push(m.clone());
                    }
                }
                turn.mode = sandbox.or(approval);
                turn.model = p
                    .get("model")
                    .and_then(Value::as_str)
                    .map(String::from)
                    .or(turn.model.take());
            }
            ("response_item", "function_call")
            | ("response_item", "local_shell_call")
            | ("response_item", "custom_tool_call")
            | (_, "function_call") => {
                let name = p.get("name").and_then(Value::as_str).unwrap_or(
                    if ptype == "local_shell_call" {
                        "local_shell"
                    } else {
                        "?"
                    },
                );
                let call_id = p
                    .get("call_id")
                    .or_else(|| p.get("id"))
                    .and_then(Value::as_str)
                    .map(String::from);
                let input: Value = match p.get("arguments") {
                    Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(Value::Null),
                    Some(o) => o.clone(),
                    None => p.get("action").cloned().unwrap_or(Value::Null),
                };
                if ptype == "custom_tool_call" || name == "apply_patch" {
                    let patch = p
                        .get("input")
                        .and_then(Value::as_str)
                        .map(String::from)
                        .or_else(|| input.get("input").and_then(Value::as_str).map(String::from))
                        .unwrap_or_default();
                    for a in
                        patch_actions(ctx, &session, name, f, lineno, &patch, turn.cwd.as_deref())
                    {
                        push(out, &mut calls, a, call_id.as_deref(), when, &turn);
                    }
                    continue;
                }
                let mut a = action(ctx, &session, name, f, lineno);
                if ptype == "local_shell_call" {
                    a.kind = ActKind::Shell;
                    a.command = command_of(&input);
                } else {
                    classify(&mut a, &input, turn.cwd.as_deref(), ctx.home);
                }
                push(out, &mut calls, a, call_id.as_deref(), when, &turn);
            }
            ("response_item", "web_search_call") => {
                let mut a = action(ctx, &session, "web_search", f, lineno);
                a.kind = ActKind::Network;
                a.status = Status::Completed;
                push(out, &mut calls, a, None, when, &turn);
            }
            ("response_item", "function_call_output")
            | ("response_item", "custom_tool_call_output") => {
                let Some(id) = p.get("call_id").and_then(Value::as_str) else {
                    continue;
                };
                let output = p.get("output").cloned().unwrap_or(Value::Null);
                let text = result_text(&output);
                let success = output.get("success").and_then(Value::as_bool);
                let code = exit_code_in(&text);
                for &i in calls.get(id) {
                    let a = &mut out.actions[i];
                    a.exit_code = a.exit_code.or(code);
                    a.status = match (success, code) {
                        (Some(false), _) => Status::Failed,
                        (_, Some(c)) if c != 0 => Status::Failed,
                        _ => Status::Completed,
                    };
                }
            }
            ("event_msg", "patch_apply_end") => {
                let id = p.get("call_id").and_then(Value::as_str).unwrap_or("");
                let ok = p.get("success").and_then(Value::as_bool).unwrap_or(true);
                if let Some(changes) = p.get("changes").and_then(Value::as_object) {
                    let known = calls.get(id).to_vec();
                    for (path, ch) in changes {
                        let op = change_op(ch);
                        let diff = ch
                            .pointer("/update/unified_diff")
                            .and_then(Value::as_str)
                            .map(diff_counts);
                        let existing = known
                            .iter()
                            .copied()
                            .find(|&i| out.actions[i].path.as_deref() == Some(Path::new(path)));
                        let i = match existing {
                            Some(i) => i,
                            None => {
                                let mut a = action(ctx, &session, "apply_patch", f, lineno);
                                a.kind = ActKind::File;
                                a.path = Some(PathBuf::from(path));
                                push(out, &mut calls, a, Some(id), when, &turn);
                                out.actions.len() - 1
                            }
                        };
                        let a = &mut out.actions[i];
                        a.op = Some(op);
                        a.diff = diff.or(a.diff);
                        a.status = if ok {
                            Status::Completed
                        } else {
                            Status::Failed
                        };
                    }
                }
            }
            ("event_msg", "mcp_tool_call_end") => {
                let inv = p.get("invocation").cloned().unwrap_or(Value::Null);
                let tool = inv.get("tool").and_then(Value::as_str).unwrap_or("?");
                let mut a = action(ctx, &session, tool, f, lineno);
                a.kind = ActKind::Mcp;
                a.server = inv.get("server").and_then(Value::as_str).map(String::from);
                let err = p.pointer("/result/Err").is_some();
                a.status = if err {
                    Status::Failed
                } else {
                    Status::Completed
                };
                push(
                    out,
                    &mut calls,
                    a,
                    p.get("call_id").and_then(Value::as_str),
                    when,
                    &turn,
                );
            }
            ("event_msg", "item_completed") | (_, "item_completed") => {
                let item = p.get("item").cloned().unwrap_or(Value::Null);
                paginated_item(
                    ctx, &session, f, lineno, &item, when, &turn, out, &mut calls,
                );
            }
            _ => {}
        }
    }
    let _ = start;
    out.session(SessionInfo {
        id: session,
        project,
        transcript: f.to_path_buf(),
        subagent,
        modes,
        started,
        exempt: false,
    });
}

fn push(
    out: &mut Read,
    calls: &mut Calls,
    mut a: Action,
    call_id: Option<&str>,
    when: Option<chrono::DateTime<chrono::Utc>>,
    turn: &Turn,
) {
    a.when = a.when.or(when);
    a.cwd = a.cwd.clone().or_else(|| turn.cwd.clone());
    a.model = a.model.clone().or_else(|| turn.model.clone());
    a.approval.mode = a.approval.mode.clone().or_else(|| turn.mode.clone());
    if let Some(id) = call_id {
        a.call_id = Some(id.to_string());
        calls.add(id, out.actions.len());
    }
    out.actions.push(a);
}

fn change_op(ch: &Value) -> FileOp {
    if ch.get("add").is_some() || ch.get("type").and_then(Value::as_str) == Some("add") {
        FileOp::Create
    } else if ch.get("delete").is_some() || ch.get("type").and_then(Value::as_str) == Some("delete")
    {
        FileOp::Delete
    } else {
        FileOp::Modify
    }
}

/// The files an apply_patch envelope names: `*** Add File:`, `*** Delete File:`,
/// `*** Update File:` (a `*** Move to:` keeps the new path).
fn patch_actions(
    ctx: &Ctx,
    session: &str,
    tool: &str,
    f: &Path,
    line: Option<u32>,
    patch: &str,
    cwd: Option<&Path>,
) -> Vec<Action> {
    let mut out: Vec<Action> = Vec::new();
    for l in patch.lines() {
        let (op, p) = if let Some(p) = l.strip_prefix("*** Add File: ") {
            (Some(FileOp::Create), p)
        } else if let Some(p) = l.strip_prefix("*** Delete File: ") {
            (Some(FileOp::Delete), p)
        } else if let Some(p) = l.strip_prefix("*** Update File: ") {
            (Some(FileOp::Modify), p)
        } else if let Some(p) = l.strip_prefix("*** Move to: ") {
            if let Some(a) = out.last_mut() {
                a.path = command::resolve(p, cwd, ctx.home).or(Some(PathBuf::from(p)));
            }
            continue;
        } else {
            if let Some(a) = out.last_mut() {
                let d = a.diff.get_or_insert((0, 0));
                if l.starts_with('+') {
                    d.0 += 1;
                } else if l.starts_with('-') {
                    d.1 += 1;
                }
            }
            continue;
        };
        let mut a = action(ctx, session, tool, f, line);
        a.kind = ActKind::File;
        a.op = op;
        a.path = command::resolve(p.trim(), cwd, ctx.home).or(Some(PathBuf::from(p.trim())));
        out.push(a);
    }
    if out.is_empty() {
        let mut a = action(ctx, session, tool, f, line);
        a.kind = ActKind::File;
        a.op = Some(FileOp::Modify);
        out.push(a);
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn paginated_item(
    ctx: &Ctx,
    session: &str,
    f: &Path,
    line: Option<u32>,
    item: &Value,
    when: Option<chrono::DateTime<chrono::Utc>>,
    turn: &Turn,
    out: &mut Read,
    calls: &mut Calls,
) {
    let status = |s: Option<&str>| match s {
        Some("completed") => Status::Completed,
        Some("failed") => Status::Failed,
        Some("declined") => Status::Denied,
        _ => Status::Unknown,
    };
    let st = item.get("status").and_then(Value::as_str);
    let id = item.get("id").and_then(Value::as_str);
    match item.get("type").and_then(Value::as_str) {
        Some("command_execution") | Some("commandExecution") => {
            let mut a = action(ctx, session, "command_execution", f, line);
            a.kind = ActKind::Shell;
            a.command = command_of(item);
            a.cwd = item.get("cwd").and_then(Value::as_str).map(PathBuf::from);
            a.exit_code = item.get("exit_code").and_then(Value::as_i64);
            a.status = status(st);
            if a.status == Status::Denied {
                deny(&mut a, None);
            }
            push(out, calls, a, id, when, turn);
        }
        Some("file_change") | Some("fileChange") => {
            let auto = item.get("auto_approved").and_then(Value::as_bool);
            let changes: Vec<(String, Value)> = match item.get("changes") {
                Some(Value::Object(o)) => o.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
                Some(Value::Array(a)) => a
                    .iter()
                    .filter_map(|c| {
                        c.get("path")
                            .and_then(Value::as_str)
                            .map(|p| (p.to_string(), c.clone()))
                    })
                    .collect(),
                _ => Vec::new(),
            };
            for (path, ch) in changes {
                let mut a = action(ctx, session, "file_change", f, line);
                a.kind = ActKind::File;
                a.path = Some(PathBuf::from(&path));
                a.op = Some(change_op(&ch));
                a.status = status(st);
                if a.status == Status::Denied {
                    deny(&mut a, None);
                } else if auto == Some(true) {
                    a.approval.decision = Decision::Allowed;
                    a.approval.source = Some("auto".into());
                }
                push(out, calls, a, id, when, turn);
            }
        }
        Some("mcp_tool_call") | Some("mcpToolCall") => {
            let mut a = action(
                ctx,
                session,
                item.get("tool").and_then(Value::as_str).unwrap_or("?"),
                f,
                line,
            );
            a.kind = ActKind::Mcp;
            a.server = item.get("server").and_then(Value::as_str).map(String::from);
            a.status = status(st);
            push(out, calls, a, id, when, turn);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn rollout_calls_outputs_patches_and_modes() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp
            .path()
            .join("rollout-2026-10-17T10-00-00-0199a213-81c0-7800-8aa1-bbab2a035a53.jsonl");
        let lines = [
            r#"{"timestamp":"2026-10-17T10:00:00Z","type":"session_meta","payload":{"id":"0199a213-81c0-7800-8aa1-bbab2a035a53","cwd":"/home/a/p"}}"#,
            r#"{"timestamp":"2026-10-17T10:00:00Z","type":"turn_context","payload":{"cwd":"/home/a/p","approval_policy":"never","sandbox_policy":{"mode":"danger-full-access"},"model":"gpt-x"}}"#,
            r#"{"timestamp":"2026-10-17T10:00:01Z","type":"response_item","payload":{"type":"function_call","name":"shell","call_id":"c1","arguments":"{\"command\":[\"bash\",\"-lc\",\"pip install requests\"]}"}}"#,
            r#"{"timestamp":"2026-10-17T10:00:02Z","type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"Exit code: 0\nWall time: 1 seconds\nOutput:\nok"}}"#,
            r#"{"timestamp":"2026-10-17T10:00:03Z","type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","call_id":"c2","input":"*** Begin Patch\n*** Delete File: old.txt\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** End Patch"}}"#,
            r#"{"timestamp":"2026-10-17T10:00:04Z","type":"event_msg","payload":{"type":"patch_apply_end","call_id":"c2","success":true,"changes":{"/home/a/p/old.txt":{"delete":{"content":""}},"/home/a/p/src/a.rs":{"update":{"unified_diff":"@@\n-x\n+y\n"}}}}}"#,
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        let home = PathBuf::from("/home/a");
        let ctx = Ctx {
            host: "codex",
            since: Utc::now() - chrono::Duration::days(3650),
            home: &home,
        };
        let r = read(&ctx, &[f]);
        assert_eq!(r.actions.len(), 3, "{:#?}", r.actions);
        let sh = &r.actions[0];
        assert_eq!(
            (sh.command.as_deref(), sh.exit_code, sh.status),
            (Some("pip install requests"), Some(0), Status::Completed)
        );
        assert_eq!(sh.approval.mode.as_deref(), Some("danger-full-access"));
        let del = r
            .actions
            .iter()
            .find(|a| a.op == Some(FileOp::Delete))
            .unwrap();
        assert_eq!(del.path.as_deref(), Some(Path::new("/home/a/p/old.txt")));
        let upd = r
            .actions
            .iter()
            .find(|a| a.op == Some(FileOp::Modify))
            .unwrap();
        assert_eq!(upd.diff, Some((1, 1)));
        assert_eq!(
            r.sessions[0].modes,
            vec!["danger-full-access".to_string(), "never".to_string()]
        );
    }
}
