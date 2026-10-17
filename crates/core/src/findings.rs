//! The eleven findings, built from the actions and from what their commands meant. A
//! finding states what happened — a command, a file, a session — and the rule behind it;
//! it never guesses why the agent did it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, Local, Utc};

use crate::model::*;
use crate::rules::Rules;
use crate::scan::{display_home, Analysis};
use crate::sheets::Sheets;

/// Up to this many evidence lines per finding.
const EVIDENCE: usize = 8;

fn short(id: &str) -> String {
    id.chars().take(8).collect()
}

fn when(t: Option<DateTime<Utc>>) -> String {
    t.map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "time not recorded".into())
}

struct Ctx<'a> {
    inv: &'a Inventory,
    sheets: &'a Sheets,
    home: &'a Path,
}

impl Ctx<'_> {
    fn name(&self, host: &str) -> String {
        self.sheets.name_of(host).to_string()
    }

    fn where_(&self, a: &Action) -> String {
        let s = self.inv.session(&a.host, &a.session);
        let project = s.and_then(|s| s.project.clone()).or_else(|| a.cwd.clone());
        let p = project
            .map(|p| display_home(&p, self.home))
            .unwrap_or_else(|| "an unknown directory".into());
        format!(
            "{} session {} in {}, {}",
            self.name(&a.host),
            short(&a.session),
            p,
            when(a.when)
        )
    }

    fn resume(&self, host: &str, session: &str) -> Option<String> {
        self.inv
            .session(host, session)
            .and_then(|s| s.resume.clone())
    }

    fn path(&self, p: &Path) -> String {
        display_home(p, self.home)
    }
}

fn base(kind: Kind, id: String, title: String, subject: String, detail: String) -> Finding {
    Finding {
        kind,
        id,
        title,
        subject,
        detail,
        host: None,
        session: None,
        path: None,
        line: None,
        evidence: Vec::new(),
        hint: None,
        command: None,
        url: None,
    }
}

fn status_word(a: &Action) -> String {
    match (a.status, a.exit_code) {
        (_, Some(c)) => format!("exit code {c}"),
        (Status::Completed, None) => "completed".into(),
        (Status::Unknown, None) => "no result recorded".into(),
        (s, None) => format!("{s:?}").to_lowercase(),
    }
}

fn call_key(a: &Action) -> String {
    a.call_id
        .clone()
        .or_else(|| a.line.map(|l| format!("l{l}")))
        .unwrap_or_else(|| when(a.when))
}

pub fn build(
    inv: &Inventory,
    an: &Analysis,
    sheets: &Sheets,
    rules: &Rules,
    now: DateTime<Utc>,
) -> Vec<Finding> {
    let c = Ctx {
        inv,
        sheets,
        home: &inv.home,
    };
    let mut out: Vec<Finding> = Vec::new();

    // ---- destructive: one per command that ran
    let mut destructive_sessions: BTreeSet<(String, String)> = BTreeSet::new();
    for (i, rule_id, shown) in &an.destructive {
        let a = &inv.actions[*i];
        if !a.ran() {
            continue;
        }
        destructive_sessions.insert((a.host.clone(), a.session.clone()));
        let rule = rules.rule(rule_id);
        let mut f = base(
            Kind::Destructive,
            format!("{}/{}/{}/{}", a.host, a.session, call_key(a), rule_id),
            format!("Destructive command ran: {shown}"),
            c.where_(a),
            format!(
                "{} ran it ({}), approval {}. Rule `{rule_id}`: {}.",
                c.name(&a.host),
                status_word(a),
                approval_word(&a.approval),
                rule.map(|r| r.label.as_str()).unwrap_or(rule_id)
            ),
        );
        f.host = Some(a.host.clone());
        f.session = Some(a.session.clone());
        f.path = Some(a.source.clone());
        f.line = a.line;
        f.evidence = vec![shown.clone()];
        f.hint = Some("Check what is missing against the project's history and your backups; the transcript line is the exact command.".into());
        f.command = c.resume(&a.host, &a.session);
        f.url = rule.and_then(|r| r.source.url.clone());
        out.push(f);
    }

    // ---- secrets in transcripts: one per transcript file
    let mut by_file: BTreeMap<(String, PathBuf), Vec<&crate::read::SecretHit>> = BTreeMap::new();
    for (host, h) in &an.secrets {
        by_file
            .entry((host.clone(), h.file.clone()))
            .or_default()
            .push(h);
    }
    for ((host, file), hits) in by_file {
        let distinct: BTreeSet<&str> = hits.iter().map(|h| h.masked.as_str()).collect();
        let first = hits.iter().filter_map(|h| h.line).min();
        let mut f = base(
            Kind::SecretInTranscript,
            format!("{host}/{}", file.display()),
            format!("{} secret(s) in clear in a {} transcript", distinct.len(), c.name(&host)),
            c.path(&file),
            format!(
                "{} credential(s) of a known shape are written in clear in this transcript — a command printed them or a file holding them was read.",
                distinct.len()
            ),
        );
        f.host = Some(host.clone());
        f.path = Some(file.clone());
        f.line = first;
        f.evidence = distinct
            .into_iter()
            .take(EVIDENCE)
            .map(String::from)
            .collect();
        f.hint = Some("Rotate these keys at their provider. The transcript keeps them until the host prunes it; delete it yourself if you do not need it.".into());
        if host == "claude-code" {
            f.url = Some("https://code.claude.com/docs/en/claude-directory".into());
        }
        out.push(f);
    }

    // ---- outside the project: one per session
    let mut outside: BTreeMap<(String, String), Vec<&Action>> = BTreeMap::new();
    for a in &inv.actions {
        if !a.touches_file() || !a.ran() {
            continue;
        }
        let Some(p) = a.path.as_deref() else { continue };
        if !p.is_absolute() {
            continue;
        }
        let Some(project) = inv
            .session(&a.host, &a.session)
            .and_then(|s| s.project.clone())
            .or_else(|| a.cwd.clone())
        else {
            continue;
        };
        // temporary directories are fair game, unless the project itself lives in one
        if p.starts_with(&project)
            || (crate::command::is_temp(p) && !crate::command::is_temp(&project))
        {
            continue;
        }
        let own = sheets
            .host(&a.host)
            .map(|h| h.ext.own_dirs.clone())
            .unwrap_or_default();
        if own.iter().any(|d| {
            let d = d
                .strip_prefix("~/")
                .map(|r| inv.home.join(r))
                .unwrap_or_else(|| PathBuf::from(d));
            p.starts_with(d)
        }) {
            continue;
        }
        outside
            .entry((a.host.clone(), a.session.clone()))
            .or_default()
            .push(a);
    }
    let outside_sessions: BTreeSet<(String, String)> = outside.keys().cloned().collect();
    for ((host, session), acts) in &outside {
        let s = inv.session(host, session);
        let project = s
            .and_then(|s| s.project.clone())
            .map(|p| c.path(&p))
            .unwrap_or_default();
        let mut f = base(
            Kind::OutsideProject,
            format!("{host}/{session}/outside"),
            format!("{} file(s) written or deleted outside {}", acts.len(), project),
            format!("{} session {}", c.name(host), short(session)),
            format!(
                "The session worked in {project}; these paths are outside it, outside temporary directories and outside {}'s own folders.",
                c.name(host)
            ),
        );
        f.host = Some(host.clone());
        f.session = Some(session.clone());
        f.path = s.map(|s| s.transcript.clone());
        f.evidence = acts
            .iter()
            .take(EVIDENCE)
            .map(|a| {
                format!(
                    "{} {}{}",
                    op_word(a.op),
                    c.path(a.path.as_deref().unwrap_or(Path::new("?"))),
                    if a.inferred { " (by a command)" } else { "" }
                )
            })
            .collect();
        f.command = c.resume(host, session);
        out.push(f);
    }

    // ---- unattended and destructive: one per session
    for s in &inv.sessions {
        let Some(u) = sheets.host(&s.host).and_then(|h| h.ext.unattended.as_ref()) else {
            continue;
        };
        let modes: Vec<&String> = s.modes.iter().filter(|m| u.modes.contains(m)).collect();
        if modes.is_empty() {
            continue;
        }
        let key = (s.host.clone(), s.id.clone());
        let d = destructive_sessions.contains(&key);
        let o = outside_sessions.contains(&key);
        if !d && !o {
            continue;
        }
        let what = match (d, o) {
            (true, true) => "a destructive command and writes outside the project",
            (true, false) => "a destructive command",
            _ => "writes outside the project",
        };
        let mut f = base(
            Kind::UnattendedDestructive,
            format!("{}/{}/unattended", s.host, s.id),
            format!("Session run without asking ({}) with {what}", modes.iter().map(|m| m.as_str()).collect::<Vec<_>>().join(", ")),
            format!("{} session {}", s.host_name, short(&s.id)),
            format!(
                "{} ran this session in `{}`, a mode in which it does not ask before acting; the session contains {what} (see the findings of the same session).",
                s.host_name,
                modes.iter().map(|m| m.as_str()).collect::<Vec<_>>().join("`, `")
            ),
        );
        f.host = Some(s.host.clone());
        f.session = Some(s.id.clone());
        f.path = Some(s.transcript.clone());
        f.url = u.source.url.clone();
        f.command = s.resume.clone();
        out.push(f);
    }

    // ---- pipe to shell: one per command that ran
    for (i, rule_id) in &an.pipes {
        let a = &inv.actions[*i];
        if !a.ran() {
            continue;
        }
        let rule = rules.rule(rule_id);
        let cmd = a.command.clone().unwrap_or_default();
        let mut f = base(
            Kind::PipeToShell,
            format!("{}/{}/{}/{}", a.host, a.session, call_key(a), rule_id),
            format!("Downloaded code run in a shell: {cmd}"),
            c.where_(a),
            format!(
                "{} ran a download straight into a shell ({}). Rule `{rule_id}`.",
                c.name(&a.host),
                status_word(a)
            ),
        );
        f.host = Some(a.host.clone());
        f.session = Some(a.session.clone());
        f.path = Some(a.source.clone());
        f.line = a.line;
        f.evidence = vec![cmd];
        f.url = rule.and_then(|r| r.source.url.clone());
        out.push(f);
    }

    // ---- deferred script: written in one session, run in a later one
    let mut seen: BTreeSet<(PathBuf, String, String)> = BTreeSet::new();
    for (i, script) in &an.scripts {
        let run = &inv.actions[*i];
        if !run.ran() {
            continue;
        }
        let Some(w) = inv.actions.iter().find(|w| {
            matches!(w.op, Some(FileOp::Create | FileOp::Modify))
                && !w.inferred
                && w.ran()
                && w.path.as_deref() == Some(script.as_path())
                && (w.host.as_str(), w.session.as_str())
                    != (run.host.as_str(), run.session.as_str())
                && match (w.when, run.when) {
                    (Some(a), Some(b)) => a < b,
                    _ => false,
                }
        }) else {
            continue;
        };
        if !seen.insert((script.clone(), run.host.clone(), run.session.clone())) {
            continue;
        }
        let mut f = base(
            Kind::DeferredScript,
            format!("{}/{}/{}", run.host, run.session, script.display()),
            format!("A script written by an agent earlier was run: {}", c.path(script)),
            c.where_(run),
            format!(
                "{} was written by {} (session {}, {}) and run by {} (session {}, {}). What it did is not in the transcript: only its name is.",
                c.path(script),
                c.name(&w.host),
                short(&w.session),
                when(w.when),
                c.name(&run.host),
                short(&run.session),
                when(run.when)
            ),
        );
        f.host = Some(run.host.clone());
        f.session = Some(run.session.clone());
        f.path = Some(script.clone());
        f.evidence = vec![run.command.clone().unwrap_or_default()];
        f.hint =
            Some("Open the script to see what it does; it may have changed since it ran.".into());
        f.url = Some("https://forum.cursor.com/t/agent-ran-destructive-delete-script-on-dev-vault-without-warning/165459".into());
        out.push(f);
    }

    // ---- packages: one per session
    let mut pk: BTreeMap<(String, String), BTreeMap<String, BTreeSet<String>>> = BTreeMap::new();
    for (i, inst) in &an.installs {
        let a = &inv.actions[*i];
        if !a.ran() {
            continue;
        }
        let e = pk
            .entry((a.host.clone(), a.session.clone()))
            .or_default()
            .entry(inst.manager.clone())
            .or_default();
        if inst.packages.is_empty() {
            e.insert("(the project's dependencies)".into());
        } else {
            e.extend(inst.packages.iter().cloned());
        }
    }
    for ((host, session), managers) in &pk {
        let n: usize = managers
            .values()
            .map(|v| v.iter().filter(|p| !p.starts_with('(')).count())
            .sum();
        let s = inv.session(host, session);
        let mut f = base(
            Kind::PackageInstall,
            format!("{host}/{session}/packages"),
            if n > 0 {
                format!("{n} package(s) installed by the agent")
            } else {
                "Project dependencies installed by the agent".into()
            },
            format!(
                "{} session {}{}",
                c.name(host),
                short(session),
                s.and_then(|s| s.project.clone())
                    .map(|p| format!(" in {}", c.path(&p)))
                    .unwrap_or_default()
            ),
            format!(
                "Installed with {}.",
                managers.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        );
        f.host = Some(host.clone());
        f.session = Some(session.clone());
        f.path = s.map(|s| s.transcript.clone());
        f.evidence = managers
            .iter()
            .map(|(m, p)| format!("{m}: {}", p.iter().cloned().collect::<Vec<_>>().join(", ")))
            .collect();
        out.push(f);
    }

    // ---- pruning soon: one per host
    let soon = now + Duration::days(7);
    let mut pr: BTreeMap<String, Vec<&Session>> = BTreeMap::new();
    for s in &inv.sessions {
        if s.prune_at.is_some_and(|t| t > now && t <= soon) {
            pr.entry(s.host.clone()).or_default().push(s);
        }
    }
    for (host, ss) in pr {
        let first = ss.iter().filter_map(|s| s.prune_at).min();
        let loc = sheets.host(&host).and_then(|h| h.locations.first());
        let setting = loc
            .and_then(|l| l.ext.retention_setting.clone())
            .unwrap_or_default();
        let file = loc
            .and_then(|l| l.ext.retention_file.clone())
            .unwrap_or_default();
        let mut f = base(
            Kind::PruningSoon,
            format!("{host}/pruning"),
            format!("{} {} session(s) will be deleted by the host within 7 days", ss.len(), c.name(&host)),
            c.name(&host),
            format!(
                "{} deletes transcripts older than its retention; the first goes on {}. After that, what the agents did in them can no longer be read.",
                c.name(&host),
                when(first)
            ),
        );
        f.host = Some(host.clone());
        f.evidence = ss
            .iter()
            .take(EVIDENCE)
            .map(|s| {
                format!(
                    "{} — {}",
                    short(&s.id),
                    s.project.as_deref().map(|p| c.path(p)).unwrap_or_default()
                )
            })
            .collect();
        if !setting.is_empty() {
            f.hint = Some(format!("Raise `{setting}` in {file} to keep them longer."));
        }
        f.url = loc.and_then(|l| l.source.url.clone());
        out.push(f);
    }

    // ---- denied: one per session
    let mut dn: BTreeMap<(String, String), Vec<&Action>> = BTreeMap::new();
    for a in inv.denied() {
        dn.entry((a.host.clone(), a.session.clone()))
            .or_default()
            .push(a);
    }
    for ((host, session), acts) in &dn {
        let s = inv.session(host, session);
        let mut f = base(
            Kind::Denied,
            format!("{host}/{session}/denied"),
            format!("{} call(s) denied", acts.len()),
            format!("{} session {}", c.name(host), short(session)),
            "Calls the host recorded as refused — by you or by a rule. The agent did not run them."
                .into(),
        );
        f.host = Some(host.clone());
        f.session = Some(session.clone());
        f.path = s.map(|s| s.transcript.clone());
        f.evidence = acts
            .iter()
            .take(EVIDENCE)
            .map(|a| describe(a, c.home))
            .collect();
        out.push(f);
    }

    // ---- what the hosts do not write
    for h in &inv.hosts {
        if !h.present {
            continue;
        }
        let sheet = sheets.host(&h.id);
        if let Some(n) = sheet.and_then(|s| s.ext.no_action_log.as_ref()) {
            let mut f = base(
                Kind::NoActionLog,
                format!("{}/no-action-log", h.id),
                format!("{} keeps no readable record of its actions", h.name),
                h.name.clone(),
                n.reason.clone(),
            );
            f.host = Some(h.id.clone());
            f.hint = n
                .exists
                .clone()
                .map(|e| format!("What exists anyway: {e}."));
            f.url = n.source.url.clone();
            out.push(f);
        } else if !h.missing.is_empty() && !h.transcripts.is_empty() {
            let mut f = base(
                Kind::PartialRecord,
                format!("{}/partial", h.id),
                format!("{}'s transcripts lack: {}", h.name, h.missing.join("; ")),
                h.name.clone(),
                format!(
                    "Lastrun shows what {} writes; these columns stay empty for it.",
                    h.name
                ),
            );
            f.host = Some(h.id.clone());
            f.url = sheet
                .and_then(|s| s.locations.first())
                .and_then(|l| l.source.url.clone());
            out.push(f);
        }
    }

    out.sort_by(|a, b| {
        (a.kind.severity().number(), a.kind).cmp(&(b.kind.severity().number(), b.kind))
    });
    out
}

fn op_word(op: Option<FileOp>) -> &'static str {
    match op {
        Some(FileOp::Create) => "created",
        Some(FileOp::Modify) => "modified",
        Some(FileOp::Delete) => "deleted",
        Some(FileOp::Read) => "read",
        None => "touched",
    }
}

pub fn approval_word(a: &Approval) -> String {
    let base = match a.decision {
        Decision::Denied => "denied",
        Decision::Allowed => "allowed",
        Decision::NotRecorded => "not recorded",
    };
    match (&a.source, &a.mode) {
        (Some(s), Some(m)) => format!("{base} ({s}, mode {m})"),
        (Some(s), None) => format!("{base} ({s})"),
        (None, Some(m)) => format!("{base} (mode {m})"),
        _ => base.into(),
    }
}

/// One line for an action: the command, or what happened to which file, or the tool.
pub fn describe(a: &Action, home: &Path) -> String {
    match a.kind {
        ActKind::Shell => a.command.clone().unwrap_or_else(|| a.tool.clone()),
        ActKind::File => {
            let verb = match a.op {
                Some(FileOp::Create) => "create",
                Some(FileOp::Modify) => "edit",
                Some(FileOp::Delete) => "delete",
                Some(FileOp::Read) => "read",
                None => "touch",
            };
            format!(
                "{verb} {}",
                a.path
                    .as_deref()
                    .map(|p| display_home(p, home))
                    .unwrap_or_else(|| "?".into())
            )
        }
        ActKind::Mcp => format!(
            "{} on {}",
            a.tool,
            a.server.clone().unwrap_or_else(|| "an MCP server".into())
        ),
        _ => a.tool.clone(),
    }
}
