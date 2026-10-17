//! Kiro CLI: `~/.local/share/kiro-cli/data.sqlite3`, opened read-only and immutable. Kiro's
//! schema is not published; it is read with the shape of its open predecessor (Amazon Q
//! Developer CLI): table `conversations(key, value)`, the key a directory, the value a
//! `ConversationState` whose history holds `tool_uses [{id, name, args}]` and
//! `tool_use_results [{tool_use_id, status}]`. The reader walks the tree for those arrays
//! rather than trusting a fixed path. No time is written per call.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

use super::{action, classify, columns, mtime, open_ro, Calls, Ctx, Read, SessionInfo};
use crate::model::Status;

pub fn read(ctx: &Ctx, files: &[PathBuf]) -> Read {
    let mut out = Read::default();
    for f in files {
        if mtime(f).is_some_and(|t| t < ctx.since) {
            continue;
        }
        match open_ro(f) {
            Ok(conn) => {
                out.files.push(f.clone());
                if let Err(e) = one(ctx, f, &conn, &mut out) {
                    out.errors.push(format!("{}: {e}", f.display()));
                }
            }
            Err(e) => out.errors.push(format!("{}: {e}", f.display())),
        }
    }
    out
}

fn walk<'a>(v: &'a Value, key: &str, found: &mut Vec<&'a Value>) {
    match v {
        Value::Object(o) => {
            for (k, x) in o {
                if k == key {
                    if let Value::Array(a) = x {
                        found.extend(a.iter());
                    }
                }
                walk(x, key, found);
            }
        }
        Value::Array(a) => a.iter().for_each(|x| walk(x, key, found)),
        _ => {}
    }
}

fn one(ctx: &Ctx, f: &Path, conn: &Connection, out: &mut Read) -> Result<(), String> {
    if columns(conn, "conversations").is_empty() {
        return Ok(());
    }
    let mut st = conn
        .prepare("SELECT key, value FROM conversations")
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| e.to_string())?;
    for (key, value) in rows.flatten() {
        out.scan(f, None, &value);
        let Ok(v) = serde_json::from_str::<Value>(&value) else {
            continue;
        };
        let session = v
            .get("conversation_id")
            .and_then(Value::as_str)
            .unwrap_or(&key)
            .to_string();
        let project = Some(PathBuf::from(&key)).filter(|p| p.is_absolute());
        let mut uses = Vec::new();
        walk(&v, "tool_uses", &mut uses);
        let mut results = Vec::new();
        walk(&v, "tool_use_results", &mut results);
        let mut calls = Calls::default();
        let before = out.actions.len();
        for u in uses {
            let name = u.get("name").and_then(Value::as_str).unwrap_or("?");
            let mut a = action(ctx, &session, name, f, None);
            a.call_id = u.get("id").and_then(Value::as_str).map(String::from);
            a.cwd = project.clone();
            let args = u.get("args").cloned().unwrap_or(Value::Null);
            classify(&mut a, &args, project.as_deref(), ctx.home);
            if let Some(id) = &a.call_id {
                if !calls.get(id).is_empty() {
                    continue; // the same call seen twice in the tree
                }
                calls.add(id, out.actions.len());
            }
            out.actions.push(a);
        }
        for r in results {
            let Some(id) = r.get("tool_use_id").and_then(Value::as_str) else {
                continue;
            };
            for &i in calls.get(id) {
                out.actions[i].status = match r.get("status").and_then(Value::as_str) {
                    Some("Success") | Some("success") => Status::Completed,
                    Some("Error") | Some("error") => Status::Failed,
                    Some("Cancelled") | Some("cancelled") => Status::Cancelled,
                    _ => Status::Unknown,
                };
            }
        }
        if out.actions.len() > before {
            out.session(SessionInfo {
                id: session,
                project,
                transcript: f.to_path_buf(),
                subagent: false,
                modes: Vec::new(),
                started: None,
                exempt: false,
            });
        }
    }
    Ok(())
}
