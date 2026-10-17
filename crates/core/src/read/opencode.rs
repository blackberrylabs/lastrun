//! OpenCode and Kilo Code: a SQLite database, opened read-only and immutable. Two schemas
//! live side by side: V1, `part.data` rows of type `tool` (`callID`, `tool`, `state`) with
//! `session.directory` and `message.data` (`modelID`, `path.cwd`); V2, `session_message.data`
//! of assistant messages whose `content[]` holds `{type: tool, id, name, state}` blocks, with
//! `session_v2.directory`. A call found in both is counted once.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde_json::Value;

use super::{
    action, classify, columns, deny, epoch, mtime, open_ro, result_text, tables, Ctx, Read,
    SessionInfo,
};
use crate::model::{Action, FileOp, Status};

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

struct Sess {
    dir: Option<PathBuf>,
    parent: bool,
    started: Option<DateTime<Utc>>,
}

fn sessions(conn: &Connection, table: &str) -> HashMap<String, Sess> {
    let cols = columns(conn, table);
    if !cols.iter().any(|c| c == "id") {
        return HashMap::new();
    }
    let dir = if cols.iter().any(|c| c == "directory") {
        "directory"
    } else {
        "NULL"
    };
    let parent = if cols.iter().any(|c| c == "parent_id") {
        "parent_id"
    } else {
        "NULL"
    };
    let created = if cols.iter().any(|c| c == "time_created") {
        "time_created"
    } else {
        "NULL"
    };
    let sql = format!("SELECT id, {dir}, {parent}, {created} FROM {table}");
    let mut map = HashMap::new();
    if let Ok(mut st) = conn.prepare(&sql) {
        let rows = st.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1).ok().flatten(),
                r.get::<_, Option<String>>(2).ok().flatten(),
                r.get::<_, Option<i64>>(3).ok().flatten(),
            ))
        });
        if let Ok(rows) = rows {
            for (id, d, p, t) in rows.flatten() {
                map.insert(
                    id,
                    Sess {
                        dir: d.map(PathBuf::from),
                        parent: p.is_some_and(|x| !x.is_empty()),
                        started: t.and_then(epoch),
                    },
                );
            }
        }
    }
    map
}

fn one(ctx: &Ctx, f: &Path, conn: &Connection, out: &mut Read) -> Result<(), String> {
    let t = tables(conn);
    let since_ms = ctx.since.timestamp_millis();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut found: Vec<(String, Action)> = Vec::new();

    // ---- V2: session_message
    if t.iter().any(|x| x == "session_message") {
        let ss = if t.iter().any(|x| x == "session_v2") {
            sessions(conn, "session_v2")
        } else {
            HashMap::new()
        };
        let mut st = conn
            .prepare("SELECT id, session_id, time_created, data FROM session_message WHERE type = 'assistant' AND time_created >= ?1")
            .map_err(|e| e.to_string())?;
        let rows = st
            .query_map([since_ms], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                    r.get::<_, String>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        for (_id, sid, created, data) in rows.flatten() {
            out.scan(f, None, &data);
            let Ok(v) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            let sess = ss.get(&sid);
            let model = v
                .pointer("/model/modelID")
                .or_else(|| v.pointer("/model/id"))
                .and_then(Value::as_str)
                .map(String::from);
            for b in v
                .get("content")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default()
            {
                if b.get("type").and_then(Value::as_str) != Some("tool") {
                    continue;
                }
                let call = b
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !call.is_empty() && !seen.insert((sid.clone(), call.clone())) {
                    continue;
                }
                let name = b.get("name").and_then(Value::as_str).unwrap_or("?");
                let when = b
                    .pointer("/time/created")
                    .and_then(Value::as_i64)
                    .and_then(epoch)
                    .or(created.and_then(epoch));
                for a in tool_actions(
                    ctx,
                    &sid,
                    name,
                    &call,
                    b.get("state"),
                    when,
                    sess.and_then(|s| s.dir.clone()),
                    model.clone(),
                    sess.is_some_and(|s| s.parent),
                    f,
                ) {
                    found.push((sid.clone(), a));
                }
            }
        }
        for (id, s) in &ss {
            if found.iter().any(|(sid, _)| sid == id) {
                out.session(SessionInfo {
                    id: id.clone(),
                    project: s.dir.clone(),
                    transcript: f.to_path_buf(),
                    subagent: s.parent,
                    modes: Vec::new(),
                    started: s.started,
                    exempt: false,
                });
            }
        }
    }

    // ---- V1: part
    if t.iter().any(|x| x == "part") {
        let ss = if t.iter().any(|x| x == "session") {
            sessions(conn, "session")
        } else {
            HashMap::new()
        };
        let has_message = t.iter().any(|x| x == "message");
        let sql = if has_message {
            "SELECT p.session_id, p.time_created, p.data, m.data FROM part p LEFT JOIN message m ON m.id = p.message_id WHERE p.time_created >= ?1"
        } else {
            "SELECT session_id, time_created, data, NULL FROM part WHERE time_created >= ?1"
        };
        let mut st = conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = st
            .query_map([since_ms], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<i64>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        let mut v1_sessions: HashSet<String> = HashSet::new();
        for (sid, created, data, mdata) in rows.flatten() {
            out.scan(f, None, &data);
            let Ok(v) = serde_json::from_str::<Value>(&data) else {
                continue;
            };
            if v.get("type").and_then(Value::as_str) != Some("tool") {
                continue;
            }
            let call = v
                .get("callID")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !call.is_empty() && !seen.insert((sid.clone(), call.clone())) {
                continue;
            }
            let m: Value = mdata
                .and_then(|d| serde_json::from_str(&d).ok())
                .unwrap_or(Value::Null);
            let cwd = m
                .pointer("/path/cwd")
                .and_then(Value::as_str)
                .map(PathBuf::from)
                .or_else(|| ss.get(&sid).and_then(|s| s.dir.clone()));
            let model = m.get("modelID").and_then(Value::as_str).map(String::from);
            let name = v.get("tool").and_then(Value::as_str).unwrap_or("?");
            let when = v
                .pointer("/state/time/start")
                .and_then(Value::as_i64)
                .and_then(epoch)
                .or(created.and_then(epoch));
            let sub = ss.get(&sid).is_some_and(|s| s.parent);
            for a in tool_actions(
                ctx,
                &sid,
                name,
                &call,
                v.get("state"),
                when,
                cwd,
                model,
                sub,
                f,
            ) {
                found.push((sid.clone(), a));
            }
            v1_sessions.insert(sid);
        }
        for id in v1_sessions {
            let s = ss.get(&id);
            out.session(SessionInfo {
                id: id.clone(),
                project: s.and_then(|s| s.dir.clone()),
                transcript: f.to_path_buf(),
                subagent: s.is_some_and(|s| s.parent),
                modes: Vec::new(),
                started: s.and_then(|s| s.started),
                exempt: false,
            });
        }
    }
    out.actions.extend(found.into_iter().map(|(_, a)| a));
    Ok(())
}

/// The actions of one tool block: one, or one per file when the tool names several.
#[allow(clippy::too_many_arguments)]
fn tool_actions(
    ctx: &Ctx,
    session: &str,
    name: &str,
    call: &str,
    state: Option<&Value>,
    when: Option<DateTime<Utc>>,
    cwd: Option<PathBuf>,
    model: Option<String>,
    subagent: bool,
    f: &Path,
) -> Vec<Action> {
    let state = state.cloned().unwrap_or(Value::Null);
    let mut a = action(ctx, session, name, f, None);
    a.call_id = (!call.is_empty()).then(|| call.to_string());
    a.when = when;
    a.cwd = cwd.clone();
    a.model = model;
    a.subagent = subagent;
    let input = state.get("input").cloned().unwrap_or(Value::Null);
    classify(&mut a, &input, cwd.as_deref(), ctx.home);
    a.status = match state.get("status").and_then(Value::as_str) {
        Some("completed") => Status::Completed,
        Some("error") => Status::Failed,
        _ => Status::Unknown,
    };
    if a.status == Status::Failed {
        // a declined permission fails the tool (Permission.DeclinedError / CorrectedError)
        let err = result_text(state.get("error").unwrap_or(&Value::Null)).to_lowercase();
        if err.contains("permission")
            && (err.contains("reject") || err.contains("declin") || err.contains("denied"))
        {
            deny(&mut a, None);
        }
    }
    let meta = state.get("metadata").cloned().unwrap_or(Value::Null);
    a.exit_code = meta.get("exit").and_then(Value::as_i64);
    if let Some(c) = a.exit_code {
        if c != 0 && a.status == Status::Completed {
            a.status = Status::Failed;
        }
    }
    if let (Some(add), Some(del)) = (
        meta.pointer("/filediff/additions").and_then(Value::as_u64),
        meta.pointer("/filediff/deletions").and_then(Value::as_u64),
    ) {
        a.diff = Some((add as u32, del as u32));
    }
    let files = meta
        .get("files")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    if files.is_empty() {
        return vec![a];
    }
    files
        .iter()
        .map(|fl| {
            let mut b = a.clone();
            if let Some(p) = fl.get("file").and_then(Value::as_str) {
                b.path =
                    crate::command::resolve(p, cwd.as_deref(), ctx.home).or(Some(PathBuf::from(p)));
            }
            b.kind = crate::model::ActKind::File;
            b.op = Some(match fl.get("status").and_then(Value::as_str) {
                Some("added") => FileOp::Create,
                Some("deleted") => FileOp::Delete,
                _ => FileOp::Modify,
            });
            if let (Some(x), Some(y)) = (
                fl.get("additions").and_then(Value::as_u64),
                fl.get("deletions").and_then(Value::as_u64),
            ) {
                b.diff = Some((x as u32, y as u32));
            }
            b
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActKind, Decision};

    #[test]
    fn both_schemas_once_each() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("opencode.db");
        let c = Connection::open(&f).unwrap();
        let now = Utc::now().timestamp_millis();
        c.execute_batch(
            "CREATE TABLE session(id TEXT, directory TEXT, parent_id TEXT, time_created INTEGER);
             CREATE TABLE message(id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE part(id TEXT, message_id TEXT, session_id TEXT, time_created INTEGER, data TEXT);
             CREATE TABLE session_v2(id TEXT, directory TEXT, parent_id TEXT, time_created INTEGER);
             CREATE TABLE session_message(id TEXT, session_id TEXT, type TEXT, seq INTEGER, time_created INTEGER, data TEXT);",
        )
        .unwrap();
        c.execute(
            "INSERT INTO session VALUES ('s1','/home/a/p',NULL,?1)",
            [now],
        )
        .unwrap();
        c.execute("INSERT INTO message VALUES ('m1','s1',?1,'{\"modelID\":\"k2\",\"path\":{\"cwd\":\"/home/a/p\"}}')", [now]).unwrap();
        c.execute(
            "INSERT INTO part VALUES ('p1','m1','s1',?1,'{\"type\":\"tool\",\"callID\":\"c1\",\"tool\":\"bash\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"cargo test\"},\"metadata\":{\"exit\":101}}}')",
            [now],
        )
        .unwrap();
        c.execute(
            "INSERT INTO part VALUES ('p2','m1','s1',?1,'{\"type\":\"tool\",\"callID\":\"c2\",\"tool\":\"edit\",\"state\":{\"status\":\"error\",\"input\":{\"filePath\":\"a.rs\"},\"error\":\"The user rejected permission to use this specific tool call.\"}}')",
            [now],
        )
        .unwrap();
        c.execute(
            "INSERT INTO session_v2 VALUES ('s1','/home/a/p',NULL,?1)",
            [now],
        )
        .unwrap();
        c.execute(
            "INSERT INTO session_message VALUES ('x','s1','assistant',1,?1,'{\"content\":[{\"type\":\"tool\",\"id\":\"c1\",\"name\":\"bash\",\"state\":{\"status\":\"completed\",\"input\":{\"command\":\"cargo test\"}}},{\"type\":\"tool\",\"id\":\"c3\",\"name\":\"apply_patch\",\"state\":{\"status\":\"completed\",\"input\":{},\"metadata\":{\"files\":[{\"file\":\"old.txt\",\"status\":\"deleted\"}]}}}]}')",
            [now],
        )
        .unwrap();
        drop(c);
        let home = PathBuf::from("/home/a");
        let ctx = Ctx {
            host: "opencode",
            since: Utc::now() - chrono::Duration::days(30),
            home: &home,
        };
        let r = read(&ctx, &[f]);
        assert_eq!(r.actions.len(), 3, "{:#?}", r.actions);
        let del = r
            .actions
            .iter()
            .find(|a| a.op == Some(FileOp::Delete))
            .unwrap();
        assert_eq!(del.path.as_deref(), Some(Path::new("/home/a/p/old.txt")));
        let edit = r.actions.iter().find(|a| a.tool == "edit").unwrap();
        assert_eq!(edit.approval.decision, Decision::Denied);
        assert!(r.actions.iter().any(|a| a.kind == ActKind::Shell));
        assert_eq!(r.sessions.len(), 1);
    }
}
