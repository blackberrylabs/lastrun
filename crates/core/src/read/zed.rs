//! Zed: `~/.local/share/zed/threads/threads.db`, opened read-only and immutable; table
//! `threads(id, updated_at, data_type, data)` where `data` is a `DbThread` as JSON or zstd.
//! Agent messages hold `ToolUse` items and a `tool_results` map. No time is written per
//! message: every action of a thread carries the thread's last update.

use std::path::{Path, PathBuf};

use rusqlite::Connection;
use serde_json::Value;

use super::{
    action, classify, columns, deny, diff_counts, exit_code_in, mtime, open_ro, result_text, time,
    Ctx, Read, SessionInfo,
};
use crate::model::Status;

/// Zed's refusal (agent/src/thread.rs L6392-6393).
const DENIED: &str = "Permission to run tool denied by user";

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
    let cols = columns(conn, "threads");
    if cols.is_empty() {
        return Ok(());
    }
    let parent = if cols.iter().any(|c| c == "parent_id") {
        "parent_id"
    } else {
        "NULL"
    };
    let sql = format!("SELECT id, updated_at, data_type, data, {parent} FROM threads");
    let mut st = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = st
        .query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Vec<u8>>(3)?,
                r.get::<_, Option<String>>(4).ok().flatten(),
            ))
        })
        .map_err(|e| e.to_string())?;
    for (id, updated, dtype, blob, parent) in rows.flatten() {
        let when = time(updated.map(Value::String).as_ref());
        if when.is_some_and(|w| w < ctx.since) {
            continue;
        }
        let bytes = if dtype.as_deref() == Some("zstd") {
            zstd::decode_all(&blob[..]).unwrap_or_default()
        } else {
            blob
        };
        let text = String::from_utf8_lossy(&bytes);
        out.scan(f, None, &text);
        let Ok(t) = serde_json::from_str::<Value>(&text) else {
            continue;
        };
        let project = t
            .pointer("/initial_project_snapshot/worktree_snapshots/0/worktree_path")
            .and_then(Value::as_str)
            .map(PathBuf::from);
        let model = t
            .pointer("/model/model")
            .and_then(Value::as_str)
            .map(String::from);
        let subagent = parent.is_some() || t.get("subagent_context").is_some_and(|x| !x.is_null());
        let mut n = 0;
        for m in t
            .get("messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default()
        {
            let Some(agent) = m.get("Agent") else {
                continue;
            };
            let results = agent.get("tool_results").cloned().unwrap_or(Value::Null);
            for c in agent
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                let Some(u) = c.get("ToolUse") else { continue };
                let name = u.get("name").and_then(Value::as_str).unwrap_or("?");
                let call = u.get("id").and_then(Value::as_str).unwrap_or("");
                let mut a = action(ctx, &id, name, f, None);
                a.call_id = (!call.is_empty()).then(|| call.to_string());
                a.when = when;
                a.model = model.clone();
                a.subagent = subagent;
                let input = u.get("input").cloned().unwrap_or(Value::Null);
                let cwd = match input.get("cd").and_then(Value::as_str) {
                    Some(cd) => crate::command::resolve(cd, project.as_deref(), ctx.home),
                    None => project.clone(),
                };
                a.cwd = cwd.clone();
                classify(&mut a, &input, cwd.as_deref(), ctx.home);
                if let Some(r) = results.get(call) {
                    let txt = result_text(r.get("content").unwrap_or(&Value::Null));
                    let err = r.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                    a.exit_code = exit_code_in(&txt);
                    if txt.contains(DENIED) {
                        deny(&mut a, Some("user"));
                    } else {
                        a.status = if err {
                            Status::Failed
                        } else {
                            Status::Completed
                        };
                    }
                    if let Some(o) = r.get("output").filter(|o| o.is_object()) {
                        if let Some(p) = o.get("input_path").and_then(Value::as_str) {
                            a.path = crate::command::resolve(p, project.as_deref(), ctx.home)
                                .or(Some(PathBuf::from(p)));
                        }
                        if let Some(d) = o.get("diff").and_then(Value::as_str) {
                            a.diff = Some(diff_counts(d));
                        }
                    }
                }
                out.actions.push(a);
                n += 1;
            }
        }
        if n > 0 {
            out.session(SessionInfo {
                id: id.clone(),
                project,
                transcript: f.to_path_buf(),
                subagent,
                modes: Vec::new(),
                started: when,
                exempt: false,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActKind, Decision};
    use chrono::Utc;

    #[test]
    fn threads_json_and_zstd() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("threads.db");
        let c = Connection::open(&f).unwrap();
        c.execute_batch("CREATE TABLE threads(id TEXT PRIMARY KEY, summary TEXT, updated_at TEXT, data_type TEXT, data BLOB);").unwrap();
        let t = r#"{"title":"x","messages":[{"Agent":{"content":[{"ToolUse":{"id":"c1","name":"terminal","input":{"command":"cargo test","cd":"."}}},{"ToolUse":{"id":"c2","name":"delete_path","input":{"path":"old"}}}],"tool_results":{"c1":{"tool_use_id":"c1","tool_name":"terminal","is_error":true,"content":[{"Text":"Command \"cargo test\" failed with exit code 101."}]},"c2":{"tool_use_id":"c2","is_error":true,"content":[{"Text":"Permission to run tool denied by user"}]}}}}],"initial_project_snapshot":{"worktree_snapshots":[{"worktree_path":"/home/a/p"}]}}"#;
        let z = zstd::encode_all(t.as_bytes(), 3).unwrap();
        let now = Utc::now().to_rfc3339();
        c.execute(
            "INSERT INTO threads VALUES ('t1','', ?1, 'zstd', ?2)",
            rusqlite::params![now, z],
        )
        .unwrap();
        drop(c);
        let home = PathBuf::from("/home/a");
        let ctx = Ctx {
            host: "zed",
            since: Utc::now() - chrono::Duration::days(30),
            home: &home,
        };
        let r = read(&ctx, &[f]);
        assert_eq!(r.actions.len(), 2);
        assert_eq!(
            (
                r.actions[0].kind,
                r.actions[0].exit_code,
                r.actions[0].status
            ),
            (ActKind::Shell, Some(101), Status::Failed)
        );
        assert_eq!(r.actions[1].approval.decision, Decision::Denied);
        assert_eq!(
            r.actions[1].path.as_deref(),
            Some(Path::new("/home/a/p/old"))
        );
    }
}
