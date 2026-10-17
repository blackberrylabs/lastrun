//! Goose: `~/.local/share/goose/sessions/sessions.db`, opened read-only and immutable.
//! `messages.content_json` holds blocks: `toolRequest {id, toolCall {value {name, arguments}}}`
//! for calls, `toolResponse {id, toolResult {value {content, isError}}}` for results; a
//! refusal is a response whose text begins with the host's own sentence. Tools are named
//! `<extension>__<tool>`; `developer__shell` and `developer__text_editor` are the built-in
//! shell and editor, anything else is a tool of an extension (an MCP server).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

use super::{
    action, classify, columns, deny, epoch, mtime, open_ro, result_text, tables, Calls, Ctx, Read,
    SessionInfo,
};
use crate::model::{ActKind, Status};

/// The text Goose returns when the person declines a tool (agents/tool_execution.rs L151).
const DECLINED: &str = "The user has declined to run this tool.";

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

fn one(ctx: &Ctx, f: &Path, conn: &Connection, out: &mut Read) -> Result<(), String> {
    let t = tables(conn);
    if !t.iter().any(|x| x == "messages") {
        return Ok(());
    }
    let mut dirs: HashMap<String, PathBuf> = HashMap::new();
    if t.iter().any(|x| x == "sessions")
        && columns(conn, "sessions").iter().any(|c| c == "working_dir")
    {
        if let Ok(mut st) = conn.prepare("SELECT id, working_dir FROM sessions") {
            if let Ok(rows) = st.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))
            }) {
                for (id, d) in rows.flatten() {
                    if let Some(d) = d {
                        dirs.insert(id, PathBuf::from(d));
                    }
                }
            }
        }
    }
    let cols = columns(conn, "messages");
    let ts = if cols.iter().any(|c| c == "created_timestamp") {
        "created_timestamp"
    } else if cols.iter().any(|c| c == "timestamp") {
        "timestamp"
    } else {
        "0"
    };
    let sql =
        format!("SELECT session_id, {ts}, content_json FROM messages ORDER BY session_id, {ts}");
    let mut st = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<i64>>(1).ok().flatten(),
                r.get::<_, String>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut calls = Calls::default();
    let mut started: HashMap<String, chrono::DateTime<chrono::Utc>> = HashMap::new();
    for (sid, t, content) in rows.flatten() {
        let when = t.and_then(epoch);
        if when.is_some_and(|w| w < ctx.since) {
            continue;
        }
        out.scan_in(f, None, Some(&sid), &content);
        if let Some(w) = when {
            started.entry(sid.clone()).or_insert(w);
        }
        let Ok(Value::Array(blocks)) = serde_json::from_str::<Value>(&content) else {
            continue;
        };
        let cwd = dirs.get(&sid).cloned();
        for b in blocks {
            match b.get("type").and_then(Value::as_str) {
                Some("toolRequest") => {
                    let call = b.get("toolCall").cloned().unwrap_or(Value::Null);
                    let call = call.get("value").cloned().unwrap_or(call);
                    let full = call
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or("?")
                        .to_string();
                    let (ext, tool) = full
                        .split_once("__")
                        .map(|(e, t)| (Some(e.to_string()), t.to_string()))
                        .unwrap_or((None, full.clone()));
                    let input = call.get("arguments").cloned().unwrap_or(Value::Null);
                    let mut a = action(ctx, &sid, &full, f, None);
                    a.when = when;
                    a.cwd = cwd.clone();
                    if ext.as_deref() == Some("developer") || ext.is_none() {
                        a.tool = tool.clone();
                        classify(&mut a, &input, cwd.as_deref(), ctx.home);
                        a.tool = full.clone();
                    } else {
                        a.kind = ActKind::Mcp;
                        a.server = ext;
                    }
                    if let Some(id) = b.get("id").and_then(Value::as_str) {
                        a.call_id = Some(id.to_string());
                        calls.add(id, out.actions.len());
                    }
                    out.actions.push(a);
                }
                Some("toolResponse") => {
                    let Some(id) = b.get("id").and_then(Value::as_str) else {
                        continue;
                    };
                    let res = b.get("toolResult").cloned().unwrap_or(Value::Null);
                    let err_status = res.get("status").and_then(Value::as_str) == Some("error");
                    let val = res.get("value").cloned().unwrap_or(res.clone());
                    let is_error =
                        val.get("isError").and_then(Value::as_bool).unwrap_or(false) || err_status;
                    let text = result_text(&val);
                    for &i in calls.get(id) {
                        let a = &mut out.actions[i];
                        if text.starts_with(DECLINED) {
                            deny(a, Some("user"));
                        } else {
                            a.status = if is_error {
                                Status::Failed
                            } else {
                                Status::Completed
                            };
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let mut ids: Vec<String> = out
        .actions
        .iter()
        .filter(|a| a.source == f)
        .map(|a| a.session.clone())
        .collect();
    ids.sort();
    ids.dedup();
    for id in ids {
        out.session(SessionInfo {
            project: dirs.get(&id).cloned(),
            transcript: f.to_path_buf(),
            subagent: false,
            modes: Vec::new(),
            started: started.get(&id).copied(),
            exempt: false,
            id,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Decision, FileOp};
    use chrono::Utc;

    #[test]
    fn requests_responses_and_refusals() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("sessions.db");
        let c = Connection::open(&f).unwrap();
        let now = Utc::now().timestamp();
        c.execute_batch("CREATE TABLE sessions(id TEXT, working_dir TEXT); CREATE TABLE messages(id INTEGER, session_id TEXT, role TEXT, content_json TEXT, created_timestamp INTEGER);").unwrap();
        c.execute("INSERT INTO sessions VALUES ('g1','/home/a/p')", [])
            .unwrap();
        let req = r#"[{"type":"toolRequest","id":"c1","toolCall":{"status":"success","value":{"name":"developer__shell","arguments":{"command":"apt list --installed"}}}},{"type":"toolRequest","id":"c2","toolCall":{"status":"success","value":{"name":"developer__text_editor","arguments":{"command":"write","path":"notes.md"}}}},{"type":"toolRequest","id":"c3","toolCall":{"status":"success","value":{"name":"github__create_issue","arguments":{}}}}]"#;
        let res = r#"[{"type":"toolResponse","id":"c1","toolResult":{"status":"success","value":{"content":[{"type":"text","text":"ok"}],"isError":false}}},{"type":"toolResponse","id":"c2","toolResult":{"status":"success","value":{"content":[{"type":"text","text":"The user has declined to run this tool."}],"isError":true}}}]"#;
        c.execute(
            "INSERT INTO messages VALUES (1,'g1','assistant',?1,?2)",
            rusqlite::params![req, now],
        )
        .unwrap();
        c.execute(
            "INSERT INTO messages VALUES (2,'g1','user',?1,?2)",
            rusqlite::params![res, now],
        )
        .unwrap();
        drop(c);
        let home = PathBuf::from("/home/a");
        let ctx = Ctx {
            host: "goose",
            since: Utc::now() - chrono::Duration::days(30),
            home: &home,
        };
        let r = read(&ctx, &[f]);
        assert_eq!(r.actions.len(), 3);
        assert_eq!(
            (r.actions[0].kind, r.actions[0].status),
            (ActKind::Shell, Status::Completed)
        );
        assert_eq!(
            (r.actions[1].op, r.actions[1].approval.decision),
            (Some(FileOp::Create), Decision::Denied)
        );
        assert_eq!(
            (r.actions[2].kind, r.actions[2].server.as_deref()),
            (ActKind::Mcp, Some("github"))
        );
    }
}
