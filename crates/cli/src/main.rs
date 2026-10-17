//! Lastrun command line: `lastrun` (the findings, most serious first), `lastrun sessions`,
//! `lastrun actions <session>` (one session's chronology), `lastrun deleted`, `lastrun
//! packages`, `lastrun denied`, `lastrun hosts`, `lastrun report` (for the Blackberrylabs
//! control tower, always JSON); `--json` for scripts. Read-only; never the text of an
//! exchange.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{bail, Result};
use bbl_hostrules::env::Env;
use chrono::{DateTime, Local, Utc};
use clap::{Parser, Subcommand};
use lastrun_core::findings::{approval_word, describe};
use lastrun_core::model::{ActKind, Action, Decision, FileOp, Inventory, Status};
use lastrun_core::scan::display_home;
use lastrun_core::{rules::Rules, scan_with, Analysis, Options};

#[derive(Parser)]
#[command(
    name = "lastrun",
    version,
    about = "What your AI agents did on this machine — commands, files, packages — read back from the hosts' own transcripts. Read-only."
)]
struct Cli {
    /// Machine-readable output.
    #[arg(long, global = true)]
    json: bool,
    /// Look at another home directory (a backup, a fixture) instead of yours.
    #[arg(long, global = true)]
    home: Option<PathBuf>,
    /// Only this host (its id: claude-code, codex, opencode…).
    #[arg(long, global = true)]
    host: Option<String>,
    /// The window, in days (default 30 — the retention of Claude Code and Gemini CLI;
    /// older transcripts may already be gone).
    #[arg(long, global = true, value_name = "DAYS", default_value_t = 30)]
    since: i64,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// The findings, most serious first (default).
    List,
    /// The sessions in the window, newest first.
    Sessions,
    /// What one session did, in order (the session id or its first characters).
    Actions {
        session: String,
        /// Also the files read, the searches, the listings.
        #[arg(long)]
        all: bool,
    },
    /// Every file deleted — by a tool or by a command.
    Deleted,
    /// Every package installed by an agent.
    Packages,
    /// Every call the host recorded as refused.
    Denied,
    /// The hosts Lastrun knows: present or not, readable or not, what their transcripts lack.
    Hosts,
    /// The inventory as a `bbl-report/1` document for the Blackberrylabs control tower
    /// (always JSON).
    Report,
}

fn main() {
    if let Err(e) = run() {
        // `lastrun sessions | head`: the reader left, nothing went wrong
        if e.chain().any(|c| {
            c.downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::BrokenPipe)
        }) {
            std::process::exit(0);
        }
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let mut env = Env::current();
    if let Some(h) = &cli.home {
        env.home = h.clone();
    }
    let started = std::time::Instant::now();
    let opt = Options {
        now: Utc::now(),
        since_days: cli.since.max(1),
    };
    let (inv, an) = scan_with(
        &env,
        &lastrun_core::sheets::embedded(),
        &Rules::embedded(),
        &opt,
    );
    let duration_ms = started.elapsed().as_millis() as u64;
    let host = cli.host.as_deref();
    let mut out = std::io::stdout().lock();
    match cli.command.unwrap_or(Command::List) {
        Command::Report => out.write_all(
            lastrun_core::report::build(&inv, duration_ms)
                .to_json()
                .as_bytes(),
        )?,
        Command::List if cli.json => writeln!(out, "{}", serde_json::to_string_pretty(&inv)?)?,
        Command::List => list(&mut out, &inv, host)?,
        Command::Sessions => sessions(&mut out, &inv, host, cli.json)?,
        Command::Actions { session, all } => actions(&mut out, &inv, &session, all, cli.json)?,
        Command::Deleted => deleted(&mut out, &inv, host, cli.json)?,
        Command::Packages => packages(&mut out, &inv, &an, host, cli.json)?,
        Command::Denied => denied(&mut out, &inv, host, cli.json)?,
        Command::Hosts => hosts(&mut out, &inv, cli.json)?,
    }
    Ok(())
}

fn t(x: Option<DateTime<Utc>>) -> String {
    x.map(|t| t.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".into())
}

fn header(out: &mut impl Write, inv: &Inventory) -> Result<()> {
    let present: Vec<&str> = inv
        .hosts
        .iter()
        .filter(|h| h.present)
        .map(|h| h.name.as_str())
        .collect();
    writeln!(
        out,
        "{} session(s), {} command(s), {} file(s) deleted since {} · hosts present: {}",
        inv.sessions.len(),
        inv.commands(),
        inv.deleted().count(),
        inv.since,
        if present.is_empty() {
            "none".into()
        } else {
            present.join(", ")
        }
    )?;
    writeln!(out)?;
    Ok(())
}

fn list(out: &mut impl Write, inv: &Inventory, host: Option<&str>) -> Result<()> {
    header(out, inv)?;
    let shown: Vec<_> = inv
        .findings
        .iter()
        .filter(|f| host.is_none_or(|h| f.host.as_deref() == Some(h)))
        .collect();
    if shown.is_empty() {
        writeln!(out, "No finding.")?;
    }
    let mut last = None;
    for f in &shown {
        let sev = f.kind.severity();
        if last != Some(sev) {
            writeln!(out, "── {} ──", sev.name().to_uppercase())?;
            last = Some(sev);
        }
        writeln!(out, "{:<13} {}", f.kind.label(), f.title)?;
        writeln!(out, "{:<13} {} — {}", "", f.subject, f.detail)?;
        for e in &f.evidence {
            writeln!(out, "{:<13}   {e}", "")?;
        }
        if let Some(p) = &f.path {
            let line = f.line.map(|l| format!(" line {l}")).unwrap_or_default();
            writeln!(out, "{:<13} in {}{line}", "", display_home(p, &inv.home))?;
        }
        if let Some(h) = &f.hint {
            writeln!(out, "{:<13} → {h}", "")?;
        }
        if let Some(c) = &f.command {
            writeln!(out, "{:<13} → {c}", "")?;
        }
    }
    writeln!(out)?;
    for e in &inv.errors {
        writeln!(out, "could not read: {e}")?;
    }
    writeln!(out, "{} finding(s). Lastrun changes nothing and shows no text of any exchange: only what the agents did.", shown.len())?;
    Ok(())
}

fn sessions(out: &mut impl Write, inv: &Inventory, host: Option<&str>, json: bool) -> Result<()> {
    let ss: Vec<_> = inv
        .sessions
        .iter()
        .filter(|s| host.is_none_or(|h| s.host == h))
        .collect();
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&ss)?)?;
        return Ok(());
    }
    header(out, inv)?;
    let w = inv.short_width();
    writeln!(
        out,
        "{:<16} {:<17} {:<w$} {:>7} {:>5} {:>5} {:>4}  project",
        "host", "last action", "session", "actions", "cmds", "files", "den."
    )?;
    for s in &ss {
        let mut flags = Vec::new();
        if s.subagent {
            flags.push("sub-agent".to_string());
        }
        if !s.modes.is_empty() {
            flags.push(format!("mode {}", s.modes.join("/")));
        }
        if let Some(p) = s.prune_at {
            flags.push(format!(
                "pruned on {}",
                p.with_timezone(&Local).format("%Y-%m-%d")
            ));
        }
        writeln!(
            out,
            "{:<16} {:<17} {:<w$} {:>7} {:>5} {:>5} {:>4}  {}{}",
            s.host_name,
            t(s.ended.or(s.started)),
            inv.short(&s.id),
            s.actions,
            s.commands,
            s.files_touched,
            s.denied,
            s.project
                .as_deref()
                .map(|p| display_home(p, &inv.home))
                .unwrap_or_else(|| "—".into()),
            if flags.is_empty() {
                String::new()
            } else {
                format!("  ({})", flags.join(", "))
            }
        )?;
    }
    writeln!(out)?;
    writeln!(
        out,
        "`lastrun actions <session>` shows what one session did, in order."
    )?;
    Ok(())
}

fn status_word(a: &Action) -> String {
    match (a.status, a.exit_code) {
        (_, Some(c)) => format!("exit {c}"),
        (Status::Completed, None) => "ok".into(),
        (Status::Failed, None) => "failed".into(),
        (Status::Denied, None) => "DENIED".into(),
        (Status::Cancelled, None) => "cancelled".into(),
        (Status::Interrupted, None) => "interrupted".into(),
        (Status::Unknown, None) => "?".into(),
    }
}

fn actions(
    out: &mut impl Write,
    inv: &Inventory,
    prefix: &str,
    all: bool,
    json: bool,
) -> Result<()> {
    let matches: Vec<_> = inv
        .sessions
        .iter()
        .filter(|s| s.id.starts_with(prefix))
        .collect();
    let s = match matches.as_slice() {
        [s] => *s,
        [] => bail!("no session starts with `{prefix}` in the window (see `lastrun sessions`, or widen `--since`)"),
        many => bail!("{} sessions start with `{prefix}`: {}", many.len(), many.iter().map(|s| s.id.as_str()).collect::<Vec<_>>().join(", ")),
    };
    let acts: Vec<&Action> = inv
        .actions
        .iter()
        .filter(|a| a.host == s.host && a.session == s.id)
        .filter(|a| all || !(a.kind == ActKind::File && a.op == Some(FileOp::Read)))
        .collect();
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&acts)?)?;
        return Ok(());
    }
    writeln!(
        out,
        "{} session {} — {}",
        s.host_name,
        s.id,
        s.project
            .as_deref()
            .map(|p| display_home(p, &inv.home))
            .unwrap_or_else(|| "project not recorded".into())
    )?;
    writeln!(out, "transcript {}", display_home(&s.transcript, &inv.home))?;
    if let Some(r) = &s.resume {
        writeln!(out, "reopen with `{r}`")?;
    }
    writeln!(out)?;
    for a in acts {
        let kind = match (a.kind, a.op) {
            (ActKind::Shell, _) => "run",
            (ActKind::File, Some(FileOp::Create)) => "create",
            (ActKind::File, Some(FileOp::Modify)) => "edit",
            (ActKind::File, Some(FileOp::Delete)) => "delete",
            (ActKind::File, _) => "read",
            (ActKind::Mcp, _) => "mcp",
            (ActKind::Network, _) => "web",
            (ActKind::Agent, _) => "agent",
            (ActKind::Other, _) => "tool",
        };
        let what = match a.kind {
            ActKind::File => format!(
                "{}{}{}",
                a.path
                    .as_deref()
                    .map(|p| display_home(p, &inv.home))
                    .unwrap_or_else(|| "?".into()),
                a.diff
                    .map(|(x, y)| format!(" (+{x} −{y})"))
                    .unwrap_or_default(),
                if a.inferred {
                    " (by the command above)"
                } else {
                    ""
                }
            ),
            _ => describe(a, &inv.home),
        };
        let appr = match a.approval.decision {
            Decision::NotRecorded if a.approval.mode.is_none() => String::new(),
            _ => format!("  [{}]", approval_word(&a.approval)),
        };
        writeln!(
            out,
            "{:<17} {:<6} {:<9} {}{}{}",
            t(a.when),
            kind,
            status_word(a),
            what,
            if a.subagent { "  (sub-agent)" } else { "" },
            appr
        )?;
    }
    Ok(())
}

fn deleted(out: &mut impl Write, inv: &Inventory, host: Option<&str>, json: bool) -> Result<()> {
    let ds: Vec<&Action> = inv
        .deleted()
        .filter(|a| host.is_none_or(|h| a.host == h))
        .collect();
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&ds)?)?;
        return Ok(());
    }
    header(out, inv)?;
    if ds.is_empty() {
        writeln!(out, "No file deleted by an agent in the window.")?;
    }
    let w = inv.short_width();
    for a in ds {
        let exists = a.path.as_deref().is_some_and(|p| p.exists());
        writeln!(
            out,
            "{:<17} {:<16} {:<w$} {}{}{}",
            t(a.when),
            inv.session(&a.host, &a.session)
                .map(|s| s.host_name.as_str())
                .unwrap_or(&a.host),
            inv.short(&a.session),
            a.path
                .as_deref()
                .map(|p| display_home(p, &inv.home))
                .unwrap_or_else(|| "?".into()),
            if a.inferred { "  (by a command)" } else { "" },
            if exists { "  (exists again)" } else { "" }
        )?;
    }
    Ok(())
}

fn packages(
    out: &mut impl Write,
    inv: &Inventory,
    an: &Analysis,
    host: Option<&str>,
    json: bool,
) -> Result<()> {
    let mut rows: Vec<(String, String, String, String, String)> = Vec::new();
    for (i, inst) in &an.installs {
        let a = &inv.actions[*i];
        if !a.ran() || host.is_some_and(|h| a.host != h) {
            continue;
        }
        let pk = if inst.packages.is_empty() {
            "(the project's dependencies)".to_string()
        } else {
            inst.packages.join(", ")
        };
        rows.push((
            t(a.when),
            a.host.clone(),
            inv.short(&a.session).to_string(),
            inst.manager.clone(),
            pk,
        ));
    }
    if json {
        let v: Vec<BTreeMap<&str, &str>> = rows
            .iter()
            .map(|(w, h, s, m, p)| {
                BTreeMap::from([
                    ("when", w.as_str()),
                    ("host", h.as_str()),
                    ("session", s.as_str()),
                    ("manager", m.as_str()),
                    ("packages", p.as_str()),
                ])
            })
            .collect();
        writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
        return Ok(());
    }
    header(out, inv)?;
    if rows.is_empty() {
        writeln!(out, "No package installed by an agent in the window.")?;
    }
    for (w, h, s, m, p) in rows {
        let h = inv
            .hosts
            .iter()
            .find(|x| x.id == h)
            .map(|x| x.name.clone())
            .unwrap_or(h);
        writeln!(
            out,
            "{w:<17} {h:<16} {s:<sw$} {m:<6} {p}",
            sw = inv.short_width()
        )?;
    }
    Ok(())
}

fn denied(out: &mut impl Write, inv: &Inventory, host: Option<&str>, json: bool) -> Result<()> {
    let ds: Vec<&Action> = inv
        .denied()
        .filter(|a| host.is_none_or(|h| a.host == h))
        .collect();
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&ds)?)?;
        return Ok(());
    }
    header(out, inv)?;
    if ds.is_empty() {
        writeln!(out, "No call recorded as refused in the window.")?;
    }
    let w = inv.short_width();
    for a in ds {
        let name = inv
            .hosts
            .iter()
            .find(|h| h.id == a.host)
            .map(|h| h.name.as_str())
            .unwrap_or(&a.host);
        writeln!(
            out,
            "{:<17} {:<16} {:<w$} {}  [{}]",
            t(a.when),
            name,
            inv.short(&a.session),
            describe(a, &inv.home),
            approval_word(&a.approval)
        )?;
    }
    writeln!(out)?;
    writeln!(out, "Only refusals a host writes down appear here; several hosts record none (`lastrun hosts`).")?;
    Ok(())
}

fn hosts(out: &mut impl Write, inv: &Inventory, json: bool) -> Result<()> {
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&inv.hosts)?)?;
        return Ok(());
    }
    for h in &inv.hosts {
        let state = match (h.present, h.readable) {
            (false, _) => "not on this machine".to_string(),
            (true, true) => format!(
                "{} session(s), {} action(s) in the window",
                h.sessions, h.actions
            ),
            (true, false) => "present, keeps no readable record of its actions".to_string(),
        };
        writeln!(out, "{:<30} {state}", h.name)?;
        if h.present {
            writeln!(out, "{:<30} {}", "", h.presence)?;
            if let Some(d) = h.retention_days {
                writeln!(out, "{:<30} deletes transcripts after {d} days", "")?;
            }
            if !h.missing.is_empty() {
                writeln!(out, "{:<30} lacks: {}", "", h.missing.join("; "))?;
            }
            if let Some(n) = &h.note {
                writeln!(out, "{:<30} {n}", "")?;
            }
        }
    }
    Ok(())
}
