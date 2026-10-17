//! GitHub Copilot CLI: `~/.copilot/session-state/<id>/events.jsonl`, one `{type, timestamp,
//! data}` per line. `tool.execution_start` opens a call (`toolCallId`, `toolName`,
//! `arguments`, `mcpServerName`), `tool.execution_complete` closes it (`success`);
//! `permission.completed` carries `result.kind` (`approved`, `denied-by-rules`,
//! `denied-interactively-by-user`), linked to its call when the request names it;
//! `session.context_changed` gives the working directory.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    action, classify, command_of, deny, mtime, read_text, time, Calls, Ctx, Read, SessionInfo,
};
use crate::model::{ActKind, Decision, Status};

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
    let session = f
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut cwd: Option<PathBuf> = None;
    let mut project: Option<PathBuf> = None;
    let mut started = None;
    let mut calls = Calls::default();
    // permission request id → (call id, what it asked)
    let mut requests: HashMap<String, (Option<String>, Value)> = HashMap::new();
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
        if when.is_some_and(|w| w < ctx.since) {
            continue;
        }
        let d = v.get("data").cloned().unwrap_or(Value::Null);
        match v.get("type").and_then(Value::as_str).unwrap_or("") {
            "session.context_changed" | "session.start" => {
                if let Some(c) = d.get("cwd").and_then(Value::as_str) {
                    cwd = Some(PathBuf::from(c));
                    project = project.or_else(|| cwd.clone());
                }
            }
            "tool.execution_start" | "tool.user_requested" => {
                let name = d.get("toolName").and_then(Value::as_str).unwrap_or("?");
                let id = d
                    .get("toolCallId")
                    .and_then(Value::as_str)
                    .map(String::from);
                if id.as_deref().is_some_and(|i| !calls.get(i).is_empty()) {
                    continue;
                }
                let mut a = action(ctx, &session, name, f, lineno);
                a.when = when;
                a.cwd = cwd.clone();
                let input = d.get("arguments").cloned().unwrap_or(Value::Null);
                if let Some(server) = d.get("mcpServerName").and_then(Value::as_str) {
                    a.kind = ActKind::Mcp;
                    a.server = Some(server.to_string());
                } else {
                    classify(&mut a, &input, cwd.as_deref(), ctx.home);
                }
                if let Some(id) = id {
                    a.call_id = Some(id.clone());
                    calls.add(&id, out.actions.len());
                }
                out.actions.push(a);
            }
            "tool.execution_complete" => {
                let Some(id) = d.get("toolCallId").and_then(Value::as_str) else {
                    continue;
                };
                let ok = d.get("success").and_then(Value::as_bool).unwrap_or(true);
                for &i in calls.get(id) {
                    let a = &mut out.actions[i];
                    if a.status != Status::Denied {
                        a.status = if ok {
                            Status::Completed
                        } else {
                            Status::Failed
                        };
                    }
                    if a.model.is_none() {
                        a.model = d.get("model").and_then(Value::as_str).map(String::from);
                    }
                }
            }
            "permission.requested" => {
                if let Some(rid) = d.get("requestId").and_then(Value::as_str) {
                    let req = d.get("permissionRequest").cloned().unwrap_or(Value::Null);
                    let call = req
                        .get("toolCallId")
                        .and_then(Value::as_str)
                        .map(String::from);
                    requests.insert(rid.to_string(), (call, req));
                }
            }
            "permission.completed" => {
                let kind = d
                    .pointer("/result/kind")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let rid = d.get("requestId").and_then(Value::as_str).unwrap_or("");
                let (call, req) = requests.remove(rid).unwrap_or((None, Value::Null));
                let idx: Vec<usize> = call
                    .as_deref()
                    .map(|c| calls.get(c).to_vec())
                    .unwrap_or_default();
                let source = match kind {
                    "denied-by-rules" => Some("rules"),
                    "denied-interactively-by-user" => Some("user"),
                    _ => None,
                };
                if idx.is_empty() && kind.starts_with("denied") {
                    // a refusal whose call never started: kept as an action of its own
                    let mut a = action(
                        ctx,
                        &session,
                        req.get("kind")
                            .and_then(Value::as_str)
                            .unwrap_or("permission"),
                        f,
                        lineno,
                    );
                    a.when = when;
                    a.cwd = cwd.clone();
                    if let Some(c) = command_of(&req) {
                        a.kind = ActKind::Shell;
                        a.command = Some(c);
                    }
                    deny(&mut a, source);
                    out.actions.push(a);
                }
                for i in idx {
                    let a = &mut out.actions[i];
                    if kind.starts_with("denied") {
                        deny(a, source);
                    } else if kind == "approved" {
                        a.approval.decision = Decision::Allowed;
                    }
                }
            }
            _ => {}
        }
    }
    out.session(SessionInfo {
        id: session,
        project,
        transcript: f.to_path_buf(),
        subagent: false,
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
    fn events_and_permissions() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("session-state/c1");
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("events.jsonl");
        let t = Utc::now().to_rfc3339();
        let lines = [
            format!(
                r#"{{"type":"session.context_changed","timestamp":"{t}","data":{{"cwd":"/home/a/p"}}}}"#
            ),
            format!(
                r#"{{"type":"tool.execution_start","timestamp":"{t}","data":{{"toolCallId":"x1","toolName":"bash","arguments":{{"command":"git push --force"}}}}}}"#
            ),
            format!(
                r#"{{"type":"tool.execution_complete","timestamp":"{t}","data":{{"toolCallId":"x1","success":true}}}}"#
            ),
            format!(
                r#"{{"type":"permission.requested","timestamp":"{t}","data":{{"requestId":"r1","permissionRequest":{{"kind":"shell","fullCommandText":"rm -rf /"}}}}}}"#
            ),
            format!(
                r#"{{"type":"permission.completed","timestamp":"{t}","data":{{"requestId":"r1","result":{{"kind":"denied-interactively-by-user"}}}}}}"#
            ),
        ];
        std::fs::write(&f, lines.join("\n")).unwrap();
        let home = PathBuf::from("/home/a");
        let ctx = Ctx {
            host: "copilot",
            since: Utc::now() - chrono::Duration::days(30),
            home: &home,
        };
        let r = read(&ctx, &[f]);
        assert_eq!(r.actions.len(), 2);
        assert_eq!(
            (r.actions[0].status, r.actions[0].cwd.as_deref()),
            (Status::Completed, Some(Path::new("/home/a/p")))
        );
        assert_eq!(
            (
                r.actions[1].approval.decision,
                r.actions[1].approval.source.as_deref()
            ),
            (Decision::Denied, Some("user"))
        );
        assert_eq!(r.actions[1].command.as_deref(), Some("rm -rf /"));
    }
}
