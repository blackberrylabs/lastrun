//! The diff of one edit, read again from its transcript when the person asks (decision R2):
//! the change the tool made to the file, secrets masked — never the exchange around it. The
//! transcript is opened read-only, the call found by its id, and only that call's edit is
//! kept. A host whose transcript does not hold the change, or whose format is not read here
//! yet, answers `None`.

use std::path::Path;

use serde_json::Value;

use crate::command;
use crate::model::Action;
use crate::read::{open_ro, read_text};

/// At most this many lines of a diff are shown.
pub const MAX_LINES: usize = 400;

/// One line of a diff: `+` added, `-` removed, ` ` kept, `@` a hunk header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub mark: char,
    pub text: String,
}

/// The edit behind an action, as diff lines, secrets masked, cut at `MAX_LINES`.
pub fn read(a: &Action) -> Option<Vec<Line>> {
    let id = a.call_id.as_deref()?;
    let raw = match a.host.as_str() {
        "claude-code" => claude(&a.source, id),
        "codex" => codex(&a.source, id, a.path.as_deref()),
        "opencode" | "kilo-code" => opencode(&a.source, id, a.path.as_deref()),
        "gemini-cli" | "qwen-code" => gemini(&a.source, id),
        _ => None,
    }?;
    let mut out: Vec<Line> = raw
        .into_iter()
        .map(|l| Line {
            mark: l.mark,
            text: command::mask(&l.text).0,
        })
        .collect();
    if out.len() > MAX_LINES {
        let more = out.len() - MAX_LINES;
        out.truncate(MAX_LINES);
        out.push(Line {
            mark: '@',
            text: format!("… {more} more lines"),
        });
    }
    (!out.is_empty()).then_some(out)
}

fn lines(mark: char, text: &str) -> Vec<Line> {
    text.lines()
        .map(|t| Line {
            mark,
            text: t.to_string(),
        })
        .collect()
}

fn replace(old: &str, new: &str) -> Vec<Line> {
    let mut v = lines('-', old);
    v.extend(lines('+', new));
    v
}

/// A unified diff as text (`@@ … @@`, `+`, `-`, ` `); file headers dropped.
fn unified(text: &str) -> Vec<Line> {
    text.lines()
        .filter(|l| !l.starts_with("---") && !l.starts_with("+++") && !l.starts_with("Index:"))
        .filter(|l| !l.starts_with("===="))
        .filter_map(|l| {
            let mut cs = l.chars();
            let m = cs.next()?;
            Some(match m {
                '+' | '-' | ' ' => Line {
                    mark: m,
                    text: cs.as_str().to_string(),
                },
                '@' => Line {
                    mark: '@',
                    text: l.to_string(),
                },
                _ => Line {
                    mark: ' ',
                    text: l.to_string(),
                },
            })
        })
        .collect()
}

/// The section of an apply_patch envelope for one file (`*** Update File: x` … up to the
/// next `*** `), or the whole envelope when the file is not named.
fn envelope(patch: &str, path: Option<&Path>) -> Vec<Line> {
    let name = path
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_string());
    let mut out = Vec::new();
    let mut inside = name.is_none();
    for l in patch.lines() {
        if let Some(rest) = ["*** Update File: ", "*** Add File: ", "*** Delete File: "]
            .iter()
            .find_map(|p| l.strip_prefix(p))
        {
            inside = name
                .as_deref()
                .is_none_or(|n| Path::new(rest.trim()).file_name().is_some_and(|f| f == n));
            continue;
        }
        if l.starts_with("*** ") {
            continue;
        }
        if inside {
            out.extend(unified(l));
        }
    }
    out
}

/// Every JSON line of a transcript that mentions the call id.
fn json_lines(f: &Path, id: &str) -> Vec<Value> {
    let Ok(text) = read_text(f) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| l.contains(id))
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

// ---- Claude Code: the tool result's `structuredPatch`, else the tool input

fn claude(f: &Path, id: &str) -> Option<Vec<Line>> {
    let vs = json_lines(f, id);
    // the result line: toolUseResult.structuredPatch (hunks with prefixed lines)
    for v in &vs {
        let has = v
            .pointer("/message/content")
            .and_then(Value::as_array)
            .is_some_and(|c| {
                c.iter()
                    .any(|b| b.get("tool_use_id").and_then(Value::as_str) == Some(id))
            });
        if !has {
            continue;
        }
        if let Some(hunks) = v
            .pointer("/toolUseResult/structuredPatch")
            .and_then(Value::as_array)
            .filter(|h| !h.is_empty())
        {
            let mut out = Vec::new();
            for h in hunks {
                let n = |k: &str| h.get(k).and_then(Value::as_i64).unwrap_or(0);
                out.push(Line {
                    mark: '@',
                    text: format!(
                        "@@ -{},{} +{},{} @@",
                        n("oldStart"),
                        n("oldLines"),
                        n("newStart"),
                        n("newLines")
                    ),
                });
                for l in h
                    .get("lines")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(s) = l.as_str() {
                        out.extend(unified(s));
                    }
                }
            }
            return Some(out);
        }
    }
    // the call itself
    for v in &vs {
        for b in v
            .pointer("/message/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            if b.get("type").and_then(Value::as_str) != Some("tool_use")
                || b.get("id").and_then(Value::as_str) != Some(id)
            {
                continue;
            }
            let i = b.get("input").cloned().unwrap_or(Value::Null);
            let s = |k: &str| i.get(k).and_then(Value::as_str);
            if let (Some(o), Some(n)) = (s("old_string"), s("new_string")) {
                return Some(replace(o, n));
            }
            if let Some(edits) = i.get("edits").and_then(Value::as_array) {
                let mut out = Vec::new();
                for e in edits {
                    let g = |k: &str| e.get(k).and_then(Value::as_str).unwrap_or("");
                    out.push(Line {
                        mark: '@',
                        text: "@@".into(),
                    });
                    out.extend(replace(g("old_string"), g("new_string")));
                }
                return Some(out);
            }
            if let Some(c) = s("content").or(s("new_source")) {
                return Some(lines('+', c));
            }
        }
    }
    None
}

// ---- Codex: the apply_patch envelope of the call

fn codex(f: &Path, id: &str, path: Option<&Path>) -> Option<Vec<Line>> {
    for v in json_lines(f, id) {
        let p = v.get("payload").unwrap_or(&v);
        if p.get("call_id").and_then(Value::as_str) != Some(id) {
            continue;
        }
        // custom_tool_call: `input` is the envelope; function_call: `arguments` (JSON) holds
        // `input`, `patch` or a `command` whose last word is the envelope
        let mut texts: Vec<String> = Vec::new();
        if let Some(i) = p.get("input").and_then(Value::as_str) {
            texts.push(i.to_string());
        }
        if let Some(args) = p.get("arguments").and_then(Value::as_str) {
            if let Ok(a) = serde_json::from_str::<Value>(args) {
                for k in ["input", "patch"] {
                    if let Some(s) = a.get(k).and_then(Value::as_str) {
                        texts.push(s.to_string());
                    }
                }
                if let Some(c) = a.get("command").and_then(Value::as_array) {
                    texts.extend(c.iter().filter_map(Value::as_str).map(String::from));
                }
            }
        }
        if let Some(t) = texts.iter().find(|t| t.contains("*** Begin Patch")) {
            return Some(envelope(t, path));
        }
    }
    None
}

// ---- OpenCode and Kilo: the tool part of the call, in the database

fn opencode(f: &Path, id: &str, path: Option<&Path>) -> Option<Vec<Line>> {
    let conn = open_ro(f).ok()?;
    let like = format!("%{id}%");
    let mut blocks: Vec<Value> = Vec::new();
    for sql in [
        "SELECT data FROM part WHERE data LIKE ?1",
        "SELECT data FROM session_message WHERE data LIKE ?1",
    ] {
        let Ok(mut st) = conn.prepare(sql) else {
            continue;
        };
        let Ok(rows) = st.query_map([&like], |r| r.get::<_, String>(0)) else {
            continue;
        };
        for d in rows.flatten() {
            let Ok(v) = serde_json::from_str::<Value>(&d) else {
                continue;
            };
            for b in v
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if b.get("id").and_then(Value::as_str) == Some(id) {
                    blocks.push(b.clone());
                }
            }
            if v.get("callID").and_then(Value::as_str) == Some(id) {
                blocks.push(v);
            }
        }
    }
    for b in blocks {
        let st = b.get("state").cloned().unwrap_or(Value::Null);
        if let Some(d) = st.pointer("/metadata/diff").and_then(Value::as_str) {
            return Some(unified(d));
        }
        let i = st.get("input").cloned().unwrap_or(Value::Null);
        let s = |k: &str| i.get(k).and_then(Value::as_str);
        if let (Some(o), Some(n)) = (s("oldString"), s("newString")) {
            return Some(replace(o, n));
        }
        if let Some(p) = s("patchText").or(s("patch")) {
            return Some(envelope(p, path));
        }
        if let Some(c) = s("content") {
            return Some(lines('+', c));
        }
    }
    None
}

// ---- Gemini CLI and Qwen Code: the result's `fileDiff`, else the call's arguments

fn gemini(f: &Path, id: &str) -> Option<Vec<Line>> {
    let mut calls: Vec<Value> = Vec::new();
    let collect = |v: &Value, calls: &mut Vec<Value>| {
        let tc = v
            .get("toolCalls")
            .or_else(|| v.pointer("/$patch/toolCalls"))
            .and_then(Value::as_array);
        for c in tc.into_iter().flatten() {
            if c.get("id").and_then(Value::as_str) == Some(id) {
                calls.push(c.clone());
            }
        }
    };
    if f.extension().is_some_and(|e| e == "json") {
        let v: Value = serde_json::from_str(&read_text(f).ok()?).ok()?;
        for m in v
            .get("messages")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            collect(m, &mut calls);
        }
    } else {
        for v in json_lines(f, id) {
            collect(&v, &mut calls);
        }
    }
    for c in &calls {
        if let Some(d) = c.pointer("/resultDisplay/fileDiff").and_then(Value::as_str) {
            return Some(unified(d));
        }
    }
    for c in &calls {
        let a = c.get("args").cloned().unwrap_or(Value::Null);
        let s = |k: &str| a.get(k).and_then(Value::as_str);
        if let (Some(o), Some(n)) = (s("old_string"), s("new_string")) {
            return Some(replace(o, n));
        }
        if let Some(c) = s("content") {
            return Some(lines('+', c));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ActKind, Approval, FileOp, Status};
    use std::path::PathBuf;

    fn act(host: &str, source: &Path, id: &str, path: &str) -> Action {
        Action {
            host: host.into(),
            session: "s".into(),
            call_id: Some(id.into()),
            tool: "edit".into(),
            kind: ActKind::File,
            command: None,
            path: Some(PathBuf::from(path)),
            op: Some(FileOp::Modify),
            inferred: false,
            server: None,
            diff: None,
            when: None,
            cwd: None,
            approval: Approval::not_recorded(),
            status: Status::Completed,
            exit_code: None,
            model: None,
            subagent: false,
            source: source.to_path_buf(),
            line: None,
        }
    }

    #[test]
    fn each_format_gives_back_the_edit_masked() {
        let tmp = tempfile::tempdir().unwrap();
        let key = format!("sk-proj-{}", "Ab3dEf6hIj9kLm2nOp5qRs8t");
        // Claude: structuredPatch on the result line wins over the input
        let c = tmp.path().join("c.jsonl");
        let lines = [
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t1","name":"Edit","input":{"file_path":"/p/a.rs","old_string":"let a = 1;","new_string":"let a = 2;"}}]}}"#.to_string(),
            format!(r#"{{"type":"user","message":{{"content":[{{"type":"tool_result","tool_use_id":"t1"}}]}},"toolUseResult":{{"structuredPatch":[{{"oldStart":3,"oldLines":1,"newStart":3,"newLines":2,"lines":["-let a = 1;","+let a = 2;","+let k = \"{key}\";"]}}]}}}}"#),
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":"t2","name":"Write","input":{"file_path":"/p/n.txt","content":"one\ntwo"}}]}}"#.to_string(),
        ];
        std::fs::write(&c, lines.join("\n")).unwrap();
        let d = read(&act("claude-code", &c, "t1", "/p/a.rs")).unwrap();
        assert_eq!(
            d[0],
            Line {
                mark: '@',
                text: "@@ -3,1 +3,2 @@".into()
            }
        );
        assert_eq!(
            d[1],
            Line {
                mark: '-',
                text: "let a = 1;".into()
            }
        );
        assert!(!d.iter().any(|l| l.text.contains(&key)), "{d:?}");
        assert_eq!(
            read(&act("claude-code", &c, "t2", "/p/n.txt"))
                .unwrap()
                .len(),
            2
        );
        assert!(read(&act("claude-code", &c, "nope", "/p/n.txt")).is_none());

        // Codex: one file's section of the envelope
        let x = tmp.path().join("x.jsonl");
        std::fs::write(&x, r#"{"type":"response_item","payload":{"type":"custom_tool_call","name":"apply_patch","call_id":"c2","input":"*** Begin Patch\n*** Delete File: old.txt\n*** Update File: src/a.rs\n@@\n-x\n+y\n*** End Patch"}}"#).unwrap();
        let d = read(&act("codex", &x, "c2", "/p/src/a.rs")).unwrap();
        assert_eq!(d.iter().map(|l| l.mark).collect::<String>(), "@-+", "{d:?}");

        // OpenCode: metadata.diff of the part
        let db = tmp.path().join("opencode.db");
        let conn = rusqlite::Connection::open(&db).unwrap();
        conn.execute_batch("CREATE TABLE part(id TEXT, data TEXT);")
            .unwrap();
        conn.execute(
            "INSERT INTO part VALUES ('p', ?1)",
            [r#"{"type":"tool","callID":"k1","tool":"edit","state":{"input":{"filePath":"a.rs","oldString":"a","newString":"b"},"metadata":{"diff":"Index: a.rs\n--- a.rs\n+++ a.rs\n@@ -1 +1 @@\n-a\n+b\n"}}}"#],
        )
        .unwrap();
        drop(conn);
        let d = read(&act("opencode", &db, "k1", "/p/a.rs")).unwrap();
        assert_eq!(d.iter().map(|l| l.mark).collect::<String>(), "@-+");

        // a host whose diffs are not read here
        assert!(read(&act("goose", &db, "k1", "/p/a.rs")).is_none());
    }
}
