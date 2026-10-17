//! The scan: which hosts are here, what their transcripts say the agents did, and what the
//! commands meant — read before they are masked, so a rule never needs a secret and a secret
//! never leaves this function. Read-only from the first line to the last.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use bbl_hostrules::env::Env;
use bbl_hostrules::read::{get, read_tree};
use bbl_hostrules::resolve::{expand_glob, project_roots, resolve};
use bbl_hostrules::rules::Format;
use chrono::{DateTime, Duration, Local, Utc};
use serde_json::Value;

use crate::command;
use crate::model::*;
use crate::read::{self, Ctx, SecretHit, SessionInfo};
use crate::rules::{Install, Rules};
use crate::sheets::{Host, Sheets};

#[derive(Debug, Clone)]
pub struct Options {
    pub now: DateTime<Utc>,
    pub since_days: i64,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            now: Utc::now(),
            since_days: 30,
        }
    }
}

/// What the commands meant, computed on the clear text and kept by index into
/// `Inventory::actions`. The findings are built from it.
#[derive(Debug, Default, Clone)]
pub struct Analysis {
    /// (action, rule id, the simple command, masked)
    pub destructive: Vec<(usize, String, String)>,
    /// (action, rule id)
    pub pipes: Vec<(usize, String)>,
    pub installs: Vec<(usize, Install)>,
    /// (action, the script it runs, resolved)
    pub scripts: Vec<(usize, PathBuf)>,
    /// (host id, hit)
    pub secrets: Vec<(String, SecretHit)>,
}

pub fn scan(env: &Env) -> Inventory {
    scan_with(
        env,
        &crate::sheets::embedded(),
        &Rules::embedded(),
        &Options::default(),
    )
    .0
}

pub fn scan_with(
    env: &Env,
    sheets: &Sheets,
    rules: &Rules,
    opt: &Options,
) -> (Inventory, Analysis) {
    let since = opt.now - Duration::days(opt.since_days);
    let mut inv = Inventory {
        home: env.home.clone(),
        since: since.with_timezone(&Local).format("%Y-%m-%d").to_string(),
        ..Default::default()
    };
    let mut analysis = Analysis::default();
    let roots = project_roots(env);
    let mut infos: Vec<(String, SessionInfo)> = Vec::new();

    for h in &sheets.hosts {
        let (present, presence) = presence(h, env);
        let mut status = HostStatus {
            id: h.id.clone(),
            name: h.name.clone(),
            present,
            presence,
            readable: !h.locations.is_empty(),
            transcripts: Vec::new(),
            sessions: 0,
            actions: 0,
            missing: h.ext.missing.clone(),
            retention_days: None,
            note: h.ext.no_action_log.as_ref().map(|n| n.reason.clone()),
        };
        if present {
            for loc in &h.locations {
                let files: Vec<PathBuf> = resolve(loc, env, &roots)
                    .into_iter()
                    .map(|r| r.path)
                    .filter(|p| p.is_file())
                    .collect();
                if files.is_empty() {
                    continue;
                }
                let ctx = Ctx {
                    host: &h.id,
                    since,
                    home: &env.home,
                };
                let r = read::read(&loc.ext.reader, &ctx, &files);
                let retention = retention_days(loc, env);
                status.retention_days = status.retention_days.or(retention);
                status.transcripts.extend(r.files.iter().cloned());
                inv.errors.extend(r.errors);
                analysis
                    .secrets
                    .extend(r.secrets.into_iter().map(|s| (h.id.clone(), s)));
                for s in r.sessions {
                    infos.push((h.id.clone(), s));
                }
                inv.actions.extend(r.actions);
            }
        }
        inv.hosts.push(status);
    }

    // ---- the window: an action with a time before it is not shown
    inv.actions.retain(|a| a.when.is_none_or(|t| t >= since));

    // ---- what the commands mean, read on the clear text; then inferred deletes; then masks
    let mut inferred: Vec<Action> = Vec::new();
    for (i, a) in inv.actions.iter().enumerate() {
        if a.kind != ActKind::Shell {
            continue;
        }
        let Some(line) = a.command.clone() else {
            continue;
        };
        let cwd = a.cwd.clone().or_else(|| project_of(&infos, a));
        if let Some(r) = rules.pipe_to_shell(&line) {
            analysis.pipes.push((i, r.id.clone()));
        }
        for tokens in command::commands(&line) {
            if let Some(r) = rules.destructive(&tokens, cwd.as_deref(), &env.home) {
                analysis
                    .destructive
                    .push((i, r.id.clone(), command::shown(&tokens.join(" "))));
            }
            if let Some(inst) = rules.install(&tokens) {
                analysis.installs.push((i, inst));
            }
            if let Some(s) = command::executed_script(&tokens) {
                if let Some(p) = command::resolve(&s, cwd.as_deref(), &env.home) {
                    analysis.scripts.push((i, p));
                }
            }
            if let Some((_, targets)) = command::rm_parts(&tokens) {
                for t in targets {
                    if t.contains('$') && !t.starts_with("$HOME") {
                        continue; // a variable we cannot know
                    }
                    let mut d = a.clone();
                    d.kind = ActKind::File;
                    d.op = Some(FileOp::Delete);
                    d.inferred = true;
                    d.command = None;
                    d.path = command::resolve(&t, cwd.as_deref(), &env.home)
                        .or_else(|| Some(PathBuf::from(&t)));
                    inferred.push(d);
                }
            }
        }
    }
    inv.actions.extend(inferred);
    for a in &mut inv.actions {
        if let Some(c) = &a.command {
            a.command = Some(command::shown(c));
        }
    }

    // ---- sessions
    let mut by_session: BTreeMap<(String, String), Vec<usize>> = BTreeMap::new();
    for (i, a) in inv.actions.iter().enumerate() {
        by_session
            .entry((a.host.clone(), a.session.clone()))
            .or_default()
            .push(i);
    }
    for ((host, id), idx) in &by_session {
        let h = sheets.host(host);
        let info = infos
            .iter()
            .find(|(hh, s)| hh == host && &s.id == id)
            .map(|(_, s)| s.clone());
        let acts: Vec<&Action> = idx.iter().map(|&i| &inv.actions[i]).collect();
        let started = acts
            .iter()
            .filter_map(|a| a.when)
            .min()
            .into_iter()
            .chain(info.as_ref().and_then(|s| s.started))
            .min();
        let ended = acts.iter().filter_map(|a| a.when).max();
        let mut modes: BTreeSet<String> = info
            .as_ref()
            .map(|s| s.modes.iter().cloned().collect())
            .unwrap_or_default();
        modes.extend(acts.iter().filter_map(|a| a.approval.mode.clone()));
        let transcript = info
            .as_ref()
            .map(|s| s.transcript.clone())
            .unwrap_or_else(|| acts[0].source.clone());
        let retention = inv
            .hosts
            .iter()
            .find(|s| &s.id == host)
            .and_then(|s| s.retention_days);
        let exempt = info.as_ref().is_some_and(|s| s.exempt);
        let prune_at = match (retention, exempt) {
            (Some(days), false) => read::mtime(&transcript).map(|m| m + Duration::days(days)),
            _ => None,
        };
        inv.sessions.push(Session {
            host: host.clone(),
            host_name: h.map(|h| h.name.clone()).unwrap_or_else(|| host.clone()),
            id: id.clone(),
            project: info
                .as_ref()
                .and_then(|s| s.project.clone())
                .or_else(|| acts.iter().find_map(|a| a.cwd.clone())),
            started,
            ended,
            modes: modes.into_iter().collect(),
            actions: acts.iter().filter(|a| !a.inferred).count(),
            commands: acts.iter().filter(|a| a.kind == ActKind::Shell).count(),
            files_touched: acts.iter().filter(|a| a.touches_file() && a.ran()).count(),
            denied: acts
                .iter()
                .filter(|a| a.approval.decision == Decision::Denied || a.status == Status::Denied)
                .count(),
            subagent: info.as_ref().is_some_and(|s| s.subagent),
            transcript,
            prune_at,
            resume: h
                .and_then(|h| h.ext.resume.as_ref())
                .map(|r| r.command.replace("{session}", id)),
        });
    }
    inv.sessions
        .sort_by_key(|s| std::cmp::Reverse(s.ended.or(s.started)));
    for s in &mut inv.hosts {
        s.sessions = inv.sessions.iter().filter(|x| x.host == s.id).count();
        s.actions = inv
            .actions
            .iter()
            .filter(|x| x.host == s.id && !x.inferred)
            .count();
    }
    // chronological, newest last
    let mut order: Vec<usize> = (0..inv.actions.len()).collect();
    order.sort_by(|&a, &b| {
        let (x, y) = (&inv.actions[a], &inv.actions[b]);
        (x.when, &x.host, &x.session, x.line).cmp(&(y.when, &y.host, &y.session, y.line))
    });
    let remap: BTreeMap<usize, usize> = order
        .iter()
        .enumerate()
        .map(|(new, &old)| (old, new))
        .collect();
    let actions = std::mem::take(&mut inv.actions);
    let mut slots: Vec<Option<Action>> = actions.into_iter().map(Some).collect();
    inv.actions = order.iter().map(|&i| slots[i].take().unwrap()).collect();
    for x in &mut analysis.destructive {
        x.0 = remap[&x.0];
    }
    for x in &mut analysis.pipes {
        x.0 = remap[&x.0];
    }
    for x in &mut analysis.installs {
        x.0 = remap[&x.0];
    }
    for x in &mut analysis.scripts {
        x.0 = remap[&x.0];
    }

    inv.short_len = crate::model::short_len(inv.sessions.iter().map(|s| s.id.as_str()));
    inv.findings = crate::findings::build(&inv, &analysis, sheets, rules, opt.now);
    (inv, analysis)
}

fn presence(h: &Host, env: &Env) -> (bool, String) {
    let mut why = Vec::new();
    for p in &h.ext.detect.paths {
        if let Some(e) = env.expand(p) {
            if expand_glob(&e).iter().any(|x| x.exists()) {
                why.push(display_home(&e, &env.home));
            }
        }
    }
    for b in &h.ext.detect.binaries {
        if env.which(b).is_some() {
            why.push(format!("`{b}` on PATH"));
        }
    }
    if why.is_empty() {
        let looked: Vec<String> = h
            .ext
            .detect
            .paths
            .iter()
            .cloned()
            .chain(h.ext.detect.binaries.iter().map(|b| format!("`{b}`")))
            .collect();
        (false, format!("not found: {}", looked.join(", ")))
    } else {
        (true, why.join(", "))
    }
}

fn project_of(infos: &[(String, SessionInfo)], a: &Action) -> Option<PathBuf> {
    infos
        .iter()
        .find(|(h, s)| h == &a.host && s.id == a.session)
        .and_then(|(_, s)| s.project.clone())
}

/// The host's retention in days, its setting read when the file says otherwise. `None` when
/// the host does not prune, or the setting turns pruning off.
fn retention_days(
    loc: &bbl_hostrules::rules::Location<crate::sheets::RunLocation>,
    env: &Env,
) -> Option<i64> {
    let default = loc.ext.retention_days?;
    let (Some(key), Some(file)) = (&loc.ext.retention_setting, &loc.ext.retention_file) else {
        return Some(default);
    };
    let Some(path) = env.expand(file) else {
        return Some(default);
    };
    let Ok((_, tree)) = read_tree(&path, Format::Json) else {
        return Some(default);
    };
    // Gemini: `general.sessionRetention.enabled: false` turns pruning off
    if let Some(parent) = key.rsplit_once('.').map(|(p, _)| p) {
        if get(&tree, &format!("{parent}.enabled")).and_then(Value::as_bool) == Some(false) {
            return None;
        }
    }
    match get(&tree, key) {
        Some(Value::Number(n)) => n.as_i64().or(Some(default)),
        Some(Value::String(s)) => {
            let s = s.trim();
            let (num, unit) = s.split_at(s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len()));
            let n: i64 = num.parse().ok()?;
            Some(match unit {
                "h" => (n + 23) / 24,
                "w" => n * 7,
                "m" => n * 30,
                _ => n,
            })
        }
        _ => Some(default),
    }
}

/// `/home/me/x` → `~/x`.
pub fn display_home(p: &Path, home: &Path) -> String {
    match p.strip_prefix(home) {
        Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => p.display().to_string(),
    }
}
